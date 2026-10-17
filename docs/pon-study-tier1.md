# pon Study — Tier-1 JIT Design (Plan 02 input)

Source: github.com/can1357/pon, clone at commit `d0a171772341178570f42422f10bee436c4f8c50`.
MIT/Apache-2.0 granted to us. All paths verified in this clone. Tier-1 code spans
four crates: `pon-jit/src/tierup.rs` (1239 L, driver), `pon-codegen/src/optimizing.rs`
(2285 L, typed codegen), `pon-runtime/src/feedback.rs` (IC substrate), `pon-runtime/src/abi.rs`
(hotness probes, OSR poll, deopt). README:29-60 matches the code; no contradictions
found (README actually *understates*: it says the small-int fast path "is landing",
but `optimizing.rs` already compiles typed `IntI64` regions).

## Tiering overview

Tier-0 baseline: everything boxed, no type specialization, correctness baseline
(`PON_TIER0_ONLY=1` forces it; `pon-codegen/src/baseline.rs`). Tier-1: single-entry
typed regions (`IntI64` fast path) inside an otherwise-boxed function, entry guards,
side-exit to a boxed "cold twin". Hotness: call threshold 16, loop back-edge
threshold 10,000 (`pon-runtime/src/abi.rs:84-92`). Dispatch is a mutable `entry`
AtomicPtr on the function object (`pon-runtime/src/object.rs:839-856`).

## Inline caches (FeedbackCell)

- **Substrate**: `pon-runtime/src/feedback.rs`. One 32-byte `FeedbackCell` per
  feedback slot, frozen layout with compile-time offset assertions
  (`FEEDBACK_CELL_*_OFFSET`, `feedback.rs:125-152`): `packed` (u32 guard/seqlock),
  `aux`, `payload` (u64), `identity` (u64), `extra` (u64).
- **Slot assignment is static**: lowering reserves one slot per specializable op —
  `LoadAttr`/`LoadMethod`/`LoadGlobal`/`CallMethod`/`CallEx` — and the `InstKind`
  statically fixes the cell interpretation (`FeedbackKind`), no runtime kind bits
  (`pon-ir/src/lower.rs:1450-1479`; `pon-ir/src/ir.rs:191` `Inst::feedback_slot`).
- **What's cached per site**: `AttrIC {type_version, kind, offset, descriptor}`
  (`feedback.rs:458-468`; offset word doubles as replay mode:
  `ATTR_DESCR_BLIND=0` vs `ATTR_DESCR_PROBE_DICT=1`, `feedback.rs:440-456`);
  `GlobalIC {dict_version, builtins_version, value_ptr}` guarded by a namespace
  version counter seeded to 1 (`abi.rs:530-556`); `CallIC {callee_identity}`.
- **Hit/miss protocol**: type-feedback cells (`packed`) use a state machine
  EMPTY→MONO→POLY→MEGA with saturating count (`next_state`, `feedback.rs:524-548`);
  IC records double `packed` as a **seqlock**: readers load `packed` (Acquire),
  payload, re-check `packed` — mismatch ⇒ torn read ⇒ slow path. Writers CAS
  `packed` to `IC_WRITING`, store payload, Release-publish the version; a failed
  claim drops the observation (benign-racy, no locks/alloc in hot path,
  `feedback.rs:88-106`). Tier-1 codegen emits **raw loads at the frozen offsets**
  — no helper call on the IC hit path.
- **Adaptability to us**: directly adaptable. This is simpler and sturdier than
  CPython's adaptiveinterpreter tables; the seqlock-on-a-word publication trick is
  the key idea worth keeping. We'd re-freeze offsets for ap-obj's header layout.
  MMTk independence: ICs never touch GC.

## Type feedback plumbing

- Tier-0 helper calls record operand `TypeTag`s (`Other/Int/IntI64/Float/Bool/Str`,
  `feedback.rs:48-61`) into the same cells (`record`, `feedback.rs:163`).
- Tier-1 compile **snapshots** feedback on the enqueueing thread — the compiler
  never reads live `FeedbackVec` cells concurrently with mutator writes
  (`tierup.rs:833-846`, `feedback_snapshot`); speculation is `Option<(TypeTag,
  TypeTag)>` per slot. Failed tier-up ⇒ `TIER_STATE_DISABLED`, no re-queue churn
  (`tierup.rs` test `failed_tier_up_disables_future_queue_attempts`).
- A metadata pass (`pon-codegen/src/infer.rs`) seeds `Inst::inferred_type` from
  annotations + feedback-informed parameters; `pon-codegen/src/region.rs` finds a
  **maximal single-entry typed region** admitting only blocks whose insts carry
  unboxable metadata; `plan_function` produces `OptimizingPlan {region,
  entry_guards, fast_path, cold_twin}` (`optimizing.rs:28-75`). Unboxing: values
  crossing region exits get re-boxed (`exits_requiring_boxing`); arithmetic guard
  failures (overflow, div-zero) side-exit to the cold twin after reboxing
  (`optimizing.rs:287`).
- **Adaptability**: adaptable in shape, but our Plan 02 hasn't settled ap-obj value
  reprs; keep the *pipeline* (feedback snapshot → metadata pass → region finder →
  plan → typed codegen) and swap the unboxed repr for whatever ap-obj picks.

## OSR mechanics

- **Trigger**: tier-0 code calls `pon_osr_poll(loop_header)` at loop back-edges
  (emitted cold-ish by `emit_osr_poll_and_transfer`, `pon-codegen/src/baseline/
  control.rs:215-293`). The probe bumps `loop_hotness`, may queue tier-up, and —
  if tier-1 with a matching OSR entry is installed — **returns the entry pointer**
  (`abi.rs:5309-5345`).
- **Transfer**: tier-0, at the same back-edge, builds an `OsrTransferBuffer` on a
  Cranelift explicit stack slot: `{loop_header: u32, live_count: u32, slots[16]}`
  (`repr(C)`, `tierup.rs:859-876`, `OSR_MAX_LIVE = 16`). Slot order is canonical:
  all locals in `LocalId` order (NULL if unbound), then SSA values live-in at the
  header (backward dataflow `osr_live_values`, `baseline.rs:2213+`) in ascending
  `Value` index. It calls the OSR entry indirectly with the buffer; the OSR body
  runs to completion and **returns the function's final result**, which tier-0
  forwards unchanged (`tierup.rs:866-880`, `control.rs:283-289`).
- **OSR entry** (`compile_osr_function`, `baseline.rs:528-620`): signature
  `buffer -> *mut PyObject`; entry block loads locals/live values out of the buffer
  at fixed offsets, then jumps into the reachable-from-header CFG subset. Functions
  with live sets > 16 are OSR-ineligible (`tierup.rs:624`).
- No frame walking, no stack maps for the transfer itself: the tier-0 frame is
  simply abandoned after the OSR call returns its result — OSR is a **call, not a
  frame transplant**. OSR entries are never GC-map-registered; their frames stay on
  the conservative scan path (`baseline.rs:615-617`).
- **Adaptability**: directly adaptable and cheap to replicate — the whole mechanism
  is ~200 lines across control.rs + tierup.rs + abi.rs. The 16-slot cap is a pon
  simplification (fixed-size buffer); for us either keep it (eligibility gate) or
  heap-allocate the buffer. Our ap-ir CFG differs but liveness over basic blocks is
  the same problem pon solves with plain backward dataflow.

## Deopt

- There is **no full deopt-to-interpreter** (there is no interpreter). Deopt =
  side-exit from the typed fast path to the function's **cold boxed twin**, which
  is baseline lowering of the same IR — so tier-0 resumes from a semantically
  identical boxed execution. State materialization is therefore just re-boxing the
  live unboxed values at the exit (`ensure_boxed_value`, `optimizing.rs:1144+`).
- Feedback loop: exits call `pon_deopt_note(function)` (`helpers.rs:232-233`,
  `abi.rs:5354-5400`): counts per epoch; at 64 (`DEOPT_THRASH_THRESHOLD`) the
  function drops to `TIER_STATE_DEFERRED` (re-tries at `threshold << epoch`,
  max shift 6), and after 8 epochs (`DEOPT_PIN_EPOCH`) it is pinned
  `TIER_STATE_DISABLED` permanently; entry pointer restored to tier-0 code, counters
  reset. Tier states: TIER0/QUEUED/TIER1/DEFERRED/DISABLED (`object.rs:801-811`).
- **Adaptability**: adaptable, *provided* we keep a boxed tier-0 of every function
  alive — that is the load-bearing design decision. If Plan 02 wants tier-1-only
  functions, cold-twin deopt must be replaced by a real state-materialization deopt
  (much harder; see problem list below).

## Background compile threading model

- One background compiler thread owned by `TierUpDriver` (`tierup.rs:65-75`,
  mpsc `CompileRequest`). Runtime probes flip `TIER_STATE_TIER0 → QUEUED` via the
  process-wide hook `pon_tierup_set_hook` (`abi.rs:5236-5278`).
- Ordering/ownership: `CompileRequest` captures function ptr + IR snapshot id +
  **feedback snapshot taken on the enqueueing thread** (never concurrent reads of
  live cells, `tierup.rs:833+`). The target function is **pinned** (address set,
  `driver.pins`) until the request retires; on completion the finished
  `Tier1Compilation` (own `JITModule`) is moved into the driver, then install is a
  Release store of the entry pointer (`TIER_STATE_QUEUED→TIER1` CAS). Failure paths
  restore tier-0 entry or disable; shutdown joins the compiler before dropping
  retained modules (`tierup.rs` SAFETY comments, `retire_queued_to_tier0`).
  `PON_SYNC_TIERUP=1` forces synchronous compile for debugging.
- **Adaptability**: directly adaptable; the pin-until-retire + snapshot-at-enqueue +
  install-by-CAS discipline is exactly right for a no-GIL runtime like ours.
  Cranelift-specific bits (JITModule per compilation) carry over since we also use
  Cranelift.

## Green Tea GC integration points (what the JIT must know)

- **Safepoints**: generated code polls at loop headers via `pon_safepoint_poll()`;
  negative status branches to the exception-exit block (`emit_safepoint_poll`,
  `baseline.rs:2133-2151`). The poll answers `pon_gc_stop_requested()` — a global
  flag + handshake phase (`pon-gc/src/handshake.rs:190-204`). Threads acknowledge
  and wait for global resume (stop-the-world, `pon-gc/src/lib.rs:1-20`).
- **Stack maps, tier-0**: conservative stack scanning with a register-flush
  trampoline at safepoints (README:60); `declare_gc_values` (`baseline.rs:622-640`)
  marks **every pointer-typed SSA value** `needs_stack_map`, so Cranelift's safepoint
  pass records `UserStackMap` per helper call; non-reference i64s are
  over-approximated as roots and filtered by the GC's pointer classifier
  (span-table membership). Dead spilled values stop pinning garbage — this was a
  real corpus bug fix (`weakref_dicts` note, `baseline.rs:622-630`).
- **Stack maps, tier-1**: a precise registry keyed by return address
  (`pon-runtime/src/stackmap.rs`): `SafepointMap{return_address, roots:
  [StackRootSlot{fp_offset}]}`; the collector walks the native **frame-pointer
  chain** (hence `preserve_frame_points=true`, README:124); frames without maps fall
  back to conservative scanning of that range (`lib.rs:172+`). A conservative-scan
  floor API (`set_conservative_scan_floor`) keeps `del x; gc.collect()` observability
  correct (`lib.rs:156-165`).
- **Scrub discipline**: helpers consuming stack-slot arrays zero dead slots between
  call and NULL-check so stale pointers don't pin (`call_pyobject_helper_consuming`,
  `baseline.rs:2153-2175`).
- **Adaptability**: the *polls + fp-chain walk + per-callsite maps* design is sound
  but **needs redesign for us**: MMTk gives us its own safepoint/handshake model and
  (with the MMTk binding) its own expectations about root reporting; our ap-gc crate
  already exists. What transfers: (a) mark-all-pointer-values-for-stack-maps during
  baseline codegen, (b) return-address-keyed map registry + fp-chain walker as the
  root-reporting shim we hand to MMTk, (c) the scrub discipline, (d) conservative
  fallback for OSR/non-registered frames.

## The 5 hardest implementation problems, in order

1. **Precise GC roots across the whole native stack (hardest).** pon needed three
   mechanisms (conservative scan, flush trampolines, precise per-callsite maps) and
   still carries a conservative fallback for OSR frames. With MMTk the collector is
   given, but *root enumeration for native frames is entirely ours*: map registry,
   fp-chain walker (requires `preserve_frame_pointers=true` and disciplined prologues),
   conservative-fallback policy, and cross-thread handshake at MMTk safepoints.
   Pon's stackmap.rs + declare_gc_values are **directly adaptable as a shim**; the
   MMTk binding contract around them **needs redesign**.
2. **OSR with unboxed state.** pon sidesteps by OSR-entering only boxed baseline or
   boxing at transfer (all-buffer, 16-slot cap). When tier-1 runs unboxed locals
   (`IntI64`), an OSR transfer must unbox guard live-ins and the buffer encoding
   must know per-slot reprs — pon has no solution for this yet (its typed regions
   are entered only at function entry via guards, never via OSR). Approach is
   adaptable for boxed OSR; **needs redesign** for typed-region OSR.
3. **Deopt beyond cold-twin.** Cold-twin deopt is only viable while every tier-1
   function retains a full boxed baseline twin and tier-1 never elides checks the
   twin would redo. The moment we want tier-1-only code, inlined calls, or elided
   bounds checks, we need true deopt: materialize interpreter/CFG-frame state at an
   arbitrary PC from a state vector (safepoint-style metadata mapping native PC →
   SSA/liveness). pon has nothing here; **needs redesign** (this is the classic V8
   problemspace and the riskiest item on Plan 02).
4. **IC correctness under mutation + no-GIL.** Version-guarded caches (type
   versions, dict versions) require every mutation path (`setattr`, `dict` store,
   `del`, class surgery) to bump the right version counter, or caches go stale-wrong
   (not just slow). pon's namespace-version scheme (`abi.rs:530-556`) covers
   globals; attribute type-versions rely on complete bump coverage in the runtime.
   Approach directly adaptable, but the **audit surface** (every mutation site in
   ap-obj) is the hard part — a missed bump is a correctness bug, not a perf one.
5. **Feedback-slot stability across tiers.** Slot numbers are assigned at lowering
   and consumed by tier-0 helpers, tier-1 raw loads, and the snapshot — any IR
   transformation (we will have lowering steps between ap-ir and machine CFG that
   pon lacks) must preserve/re-map slots or the tier-1 raw loads read the wrong
   cell. Pon's is directly adaptable as a discipline (assign at final lowering only,
   assert-density like their `debug_assert_suspend_numbering`); adapting it to our
   extra lowering stages **needs modest redesign**.

Summary: 1 and 3 need real design work regardless of pon; 2, 4, 5 are adaptable
with known extensions. Pon's tier-1 substrate (cells, seqlock ICs, OSR-by-call,
background pin/snapshot/install) is the right skeleton for Plan 02 and can be
adopted nearly wholesale at the contract level, with attribution.
