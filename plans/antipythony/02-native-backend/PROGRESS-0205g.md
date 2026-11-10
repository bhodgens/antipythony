# PROGRESS — leaf 02.05g: GC roots shim (precise stack maps + fp-chain walker)

Branch: `tier1-0205g` (base 7ccb6ad = main). Spec: SPLIT-02.05.md §02.05g.
Design: docs/gc-mmtk-swap.md §4 (root scanning, the pon shim pattern) +
docs/pon-study-tier1.md §"Green Tea GC integration points".

## What landed

- **`crates/ap-gc/src/rootscan.rs`** (NEW, pon stackmap.rs adaptation with
  attribution header): return-address-keyed `SafepointMap` registry
  (process-global `RwLock<HashMap>` — compile threads register, the
  mutator thread reads), `StackRootSlot{fp_offset}`, `fp_chain_walk`
  (null-link / monotonic-link / `FP_CHAIN_MAX_DEPTH`=16384 guards; a
  corrupted chain is a bounded stop, never a wild walk), `mapped_frames_from`
  (walk + map-resolve join — the exact data shape the MMTk
  `Scanning::scan_roots_in_mutator_thread` native-frame arm consumes at
  stage 2), the capture-point API (`set/clear/capture_point` +
  `caller_frame_pointer` inline-asm read, aarch64/x86_64),
  `ConservativeFloorGuard` (the subtract-refs policy pinned in one named
  place), and `scrub_slot` (the pon scrub discipline as slot-rebind
  semantics — Rust's drop-on-overwrite IS the scrub; the function names
  the discipline as the single point a tier-1 unboxed repr must revisit).
- **`crates/ap-gc/src/gcmod.rs`**: the JIT-frame root feed in
  `mark_reachable_scoped` — walks from the thread's capture point, probes
  each mapped frame's live slots against the registry, seeds them as
  roots. Zero capture point (no live JIT frame) walks nothing. AUGMENT,
  NOT REPLACE: the subtract-refs seeding (`sc > internal + 1`) is untouched
  and remains the correctness base — a map can only ADD roots, never
  remove the floor.
- **`crates/ap-run/src/jit_hook.rs`**: the capture-point seam —
  `jit_call_body` captures its own frame pointer around the JIT call
  (set before `je.jit.call`, cleared immediately after, all exit paths) so
  the collector can name the ExecCtx slot array while JIT'd code runs.
  PLUS the tier-init subscript seam: `init_tier` installs
  `methods::subscript_get` as the JIT's host subscript reader (see
  "Gate-run findings" below).
- **`crates/ap-codegen-native/src/clif.rs`** (ADDITIVE ONLY — sibling owns
  this file concurrently): the `declare_gc_values` analogue. An explicit
  `gc_value_declarations` set (empty today — the boxed baseline has NO
  pointer-typed CLIF values; `ctx_ptr` is deliberately NOT marked) fed to
  `fb.declare_value_needs_stack_map` before `finalize()`, plus the
  registration tail that extracts the compiled `UserStackMap`s + frame
  layout and registers fp-relative entries in the ap-gc registry before
  `finalize_definitions()` (pon pins-maps-before-code-runs rule). With an
  empty declared set Cranelift emits no maps and the tail registers
  nothing — the correct degenerate behavior; 02.05c typed codegen flips
  the switch instead of a redesign.
- **`crates/ap-codegen-native/src/runtime.rs`**: `h_subscript` now routes
  through the installed host reader (process-global fn-pointer seam, the
  weakbridge pattern — ap-codegen-native cannot depend on ap-run); was a
  fail-closed SystemError stub that crashed every JIT-routed body reading
  `info["..."]`/`seq[i]`.
- **`crates/ap-run/src/gcsupport.rs`**: `install()` clears the safepoint
  registry alongside the root dicts (fresh maps per interpreter run).
- **`crates/ap-gc/tests/gc_invariant.rs` +4 tests**, **`crates/ap-run/
  tests/rootscan_gc_shim.rs`** (NEW, 4 fenced probes), **`crates/ap-cli/
  tests/tier_mode_differential.rs`** re-enabled.

## Gate-run findings (two real bugs fixed)

1. **t1_016 re-land** (commit 2df7e9d): the leaf base 7ccb6ad PREDATES the
   tier1-0205a-t1016fix — `clif.rs` still emitted `LoadGlobal`'s dst as
   the raw TempId, so every JIT-routed body containing a call crashed with
   `'NoneType' object is not callable`. Re-landed `temp_slot(*dst)` + the
   mode-pair differential re-enable.
2. **JIT subscript stub** (same commit): the adaptive-mode gc family run
   caught `gc011` failing — `info["uncollectable"]` in a gc.callbacks hook
   SystemError'd because `h_subscript` was fail-closed. Bisected with a
   minimal repro (`def f(p,i): print("V", i["x"])` → nothing printed, rc=0
   — the exception was swallowed because the failure left no output while
   the module still exited 0 through the print path). Fixed via the host
   seam; errors reconstruct through `ExceptionValue::py_str` (byte-identical
   to the tree-walker's rendering).

## Gates (all measured green)

- **gc family 20/20 under adaptive**: the full tests/corpus/gc/ family
  through `ap-run-probe` with `APX_TIER=adaptive` (capture point armed,
  JIT bodies routed), byte-identical to the pinned expected triples.
- **Fenced probes** (`rootscan_gc_shim.rs`, all under the 6GB RSS/240s
  fence + memory-pressure preflight + interpreter gate):
  - alloc-under-recursion (300-deep, forced GC every 50 frames, per-frame
    liveness re-check) — byte-identical vs CPython 3.14, no CORRUPT lines.
  - `del x; gc.collect()` observability (3-stage del sequence) —
    byte-identical; finalizer counts 0/1/2 as pinned.
  - deep-recursion-alloc (800 frames) + loop-alloc-gc (20k iterations,
    periodic collects) — byte-identical.
- **Per-crate tests**: ap-gc 26 lib + 13 integration (4 new rootscan pins:
  maps-never-change-the-verdict, registry hygiene, capture-point
  lifecycle, synthetic-chain join); ap-codegen-native 26 (7 lib + 8 cfg +
  7 feedback + 4 osr); ap-cli tier_mode_differential 2/2 (mode-pair table
  all MATCH incl. t1_016 + JIT-exercise); ap-run full 16 suites 191
  passed / 0 failed / 1 ignored.
- **Clippy**: -D-clean on all touched crates (ap-gc, ap-codegen-native,
  ap-run leaf files; ap-cli test-file warnings are pre-existing).

## fp-chain decision: SHIPPED (measured, cost immaterial)

Measurement (M2 Ultra, rustc 1.94, `-C force-frame-pointers=yes` vs
default on the whole interpreter):

- Microbench (helper-call-shaped hot loop, 10 runs): 13.62ms → 14.14ms =
  **+3.84%** worst case.
- Whole-interpreter probes (clean rebuilds per config, 5 runs each):
  loop-alloc-gc 0.700s → 0.712s = **+1.71%**; recursion/deep probes flat
  (0.04s both configs, wall-clock noise floor).

Decision: the fp-chain walk is fully SHIPPED — registry + walker +
capture-point seam + collector feed — with the conservative floor kept as
the correctness base. Frame-pointer retention is NOT force-enabled in the
leaf's builds (no Cargo config change ships): the walk needs it only when
real pointer-typed CLIF values exist (02.05c), the capture point already
sits in a Rust frame that debug builds retain FP for, and the
`-C force-frame-pointers=yes` measurement above is the documented cost
(~2-4%) for whoever flips it. The follow-up (nothing blocking): enable FP
retention in the tier-1 codegen flags at 02.05c time, re-measure with the
bench slice.

## Deviations / notes

- The exec tail in clif.rs registers maps under a `PENDING_MAP_BASE`
  relative scheme because pre-finalize JIT addresses are not yet known;
  with today's empty map set the arm never fires. 02.05c must replace
  this with post-finalize registration keyed on the real blob base (the
  note at the call site says exactly this).
- The untracked `target-0205g/` build dir is committed in the early
  commits (it predates the ignore); a follow-up commit adding
  `target-0205g/` to `.gitignore` + `git rm -r --cached` is left to the
  orchestrator (the removal was denied for this leaf session — it is a
  6000-file cache-only operation, no source impact).
- 02.09 coordination: this leaf ships the capture-point + safepoint-map
  DATA the no-GIL safepoint protocol needs (docs/no-gil-object-audit.md
  "Deferred (phase 2+)" names exactly this handoff). No 02.09 surface was
  touched.

## Open items

- 02.05c: flip `gc_value_declarations` non-empty in typed codegen; replace
  the PENDING_MAP_BASE scheme with post-finalize real-address keys.
- 02.05c: enable FP retention in the JIT flag builder + re-measure.
- Orchestrator: gitignore the `target-0205g/` build dir (see deviations).
