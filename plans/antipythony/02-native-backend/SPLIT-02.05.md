# SPLIT-02.05 — tier-1 (inline caches / OSR / deopt) decomposition

Design leaf, read-only output. Base inputs:
- docs/pon-study-tier1.md (pon tier-1 study @1237c75; its "5 hardest implementation
  problems" section IS the risk backbone of this split)
- docs/ic-version-audit.md (Risk-4 mutation-site → bump table, 18 rows; rows
  10/11/13/15/16/18 PENDING — counters land with the tier-1 wave)
- leaves/02.05-native-codegen.md (original T1–T12 task table)
- PROGRESS-0205.md (worktree leaf-0205 partial landing — verified at HEAD b08fff0)
- orchestrator.md §02.05/§02.06/§02.09 + Dispatch notes

## What already landed (do NOT re-scope)

Verified in tree at b08fff0 (crates/ap-codegen-native/):

| Landed | Commit | Notes |
|---|---|---|
| cfg.rs (ap-ir → cfg::Func + verifier) | 5d5c9e4 | 910 L |
| clif.rs boxed-Value baseline (cfg → CLIF → Cranelift JIT) | b565e41/ecc097e | 661 L; SlotPlan::assign is THE feedback-slot stage (Risk 5 discipline: slots assigned only at final lowering) |
| feedback.rs substrate (TypeTag, FeedbackCell, record/read/snapshot) | b0f6c81 | 279 L |
| audit_probe.rs (APX_IC_AUDIT=1 cross-check) | b0f6c81 | Risk-4 probe harness |
| osr.rs boxed OSR plan + eligibility (OsrTransferBuffer, OSR_MAX_LIVE=16) | b0f6c81 | plan only — no transfer emission, no OSR entry compile |
| worker_stack.rs (sized-stack worker, debug-overflow fix) | b0f6c81 | |
| LICENSES/ entries | 52c0c3c | T10 done; attribution headers required on first pon-DERIVED code |

Open per PROGRESS-0205.md "honest remainder": ap-run entry hook (the JIT is
NOT wired into ap-run/src/interp.rs `run_module` yet — interp.rs is still the
pure tree-walking executor), typed regions, tier-up driver, OSR transfer
emission, deopt, bench gate.

## Sub-leaves

Naming: 02.05a…02.05h. Each is one-256K-leaf dispatchable. File ownership is
exclusive per sub-leaf where possible (multi-agent-safe).

---

### 02.05a — Entry hook + tier transparency (the seam)

**Scope:** Route ap-run function bodies through the JIT: ap-run/src/interp.rs
`run_module`/`exec_block` gains a lowered-function fast path (JitFunc call via
crates/ap-codegen-native/src/runtime.rs HostCallbacks surface); boxes flow
unchanged (tier-0-native is still all-boxed, per runtime.rs's own comment that
tier-1 typed regions are the ONLY future home of unboxed values). Complete
ap-conform `--tier adaptive` (main.rs:88 already validates the flag;
crates/ap-cli/src/bin/ap_run_eval.rs honors only `--tier 0` today — wire the
adaptive mode through it). Forced-tier-0 vs adaptive byte-identity differential
family (tests/corpus/tier1-transparency/, 20 probes exist, 19/20 green tier-0
with 2 pre-existing fails unrelated to codegen).

**Files:** crates/ap-run/src/{interp.rs,lib.rs}; crates/ap-cli/src/bin/
ap_run_eval.rs; crates/ap-conform/src/main.rs. (EXCLUSIVE ownership of interp.rs
for the split's duration — other sub-leaves must not touch it.)

**Risks:** interp.rs is the hottest-file in the repo (every other campaign
touches it) — keep the diff to a single dispatch arm; GIL v1 coexistence (JIT
calls run under the same lock; no new thread state).

**Gate:** full corpus byte-identical in `--tier 0` AND `--tier adaptive`
(before any tier-1 exists adaptive == tier-0-native, so this is a pure-plumbing
gate); cargo test -p ap-codegen-native green.

**Deps:** none (dispatchable NOW; substrate all landed).

---

### 02.05b — Version counters + IC population (Risk 4 surface)

**Scope:** Land the PENDING rows of docs/ic-version-audit.md: type-version +
class-surgery version counters on ap-obj type objects (audit rows 10, 11, 13,
15, 16, 18); bind the bump sites in ap-run/src/classes.rs (type.__setattr__,
class surgery, `__set_name__`/`__init_subclass__`, super_stack route, gc
finalizer surgery, builtins-shadow bootstrap). Then populate real caches at
the four site kinds: AttrIC/GlobalIC/CallIC analogues consumed by baseline
codegen (clif.rs emit at SlotPlan sites) + tier-0 interpreter record calls
(already hook-shaped per original T3). Run the audit's Risk-4 probe matrix
(mutate-through-every-path after warm; APX_IC_AUDIT=1 cross-check).

**Files:** crates/ap-obj/src/{object.rs,dict.rs(audit-rule enforcement only —
mutation_epoch is already the dict-version source of truth, DO NOT add a
parallel counter)}; crates/ap-run/src/{classes.rs,frame.rs,methods.rs};
crates/ap-codegen-native/src/{feedback.rs,clif.rs}; docs/ic-version-audit.md
(rows PENDING→COVERED).

**Risks:** pon-study-tier1 problem #4 verbatim — a missed bump is a
CORRECTNESS bug, not a perf one; audit rows must land WITH their counters or
review rejects the path (audit rule 2). NOTE the sequencing interaction with
02.09: this sub-leaf and 02.09's object-state audit touch the SAME mutation
paths in classes.rs — see the ordering section below.

**Gate:** ic-version-audit rows all COVERED (or documented-N/A); audit probe
matrix green; corpus green in both tier modes (IC hits must be
observation-invisible); no GIL-ordering assumptions (counters atomic — they
must be correct under 02.09 later, so no per-GIL shortcuts).

**Deps:** 02.05a (IC observation only matters once adaptive mode exists to
catch divergence).

---

### 02.05c — Typed regions + guards + cold twin (the tier-1 core)

**Scope:** The pon pipeline adapted: feedback snapshot (already in
feedback.rs::snapshot) → metadata/infer pass (inferred types from TypeTag
feedback + annotations) → region finder (maximal single-entry unboxable
region) → OptimizingPlan {region, entry_guards, fast_path, cold_twin} → typed
codegen in optimizing.rs. Unboxed repr: IntI64 first (ap-obj/src/int.rs Smi
domain), guard failure → re-box → side-exit to cold twin (study
optimizing.rs:287 analogue). Cold twin = the already-landed boxed baseline of
the same cfg::Func (load-bearing invariant: baseline stays mandatory —
PROGRESS-0205 Risk-3 note).

**Files:** CREATE crates/ap-codegen-native/src/{optimizing.rs,region.rs,infer.rs};
modify crates/ap-codegen-native/src/{clif.rs,runtime.rs}. No ap-run files.

**Risks:** pon problem #5 (slot stability across lowering stages) — slots are
already structurally final-lowering-only (clif.rs:3-5); add the
assert-density analogue of pon's `debug_assert_suspend_numbering`. Unboxed
repr choice is the one genuinely open design call (study says keep the
pipeline, swap the repr for whatever ap-obj picks) — decide in-leaf, document
in the leaf report.

**Gate:** arithmetic/string micro-corpus slice: adaptive byte-identical AND
measurably faster than tier-0-native on the typed-region probes; guards
side-exit correctly (deliberate type-flip probes); corpus full green both
modes.

**Deps:** 02.05a + 02.05b (typed specialization consumes feedback snapshots;
guards need version counters to guard against).

---

### 02.05d — Deopt: cold-twin side-exit + thrash backoff (Risk 3, bounded)

**Scope:** pon-analogue deopt ONLY (cold-twin): exits re-box live unboxed
values (ensure_boxed_value analogue) and branch to the boxed twin;
deopt.rs + `deopt_note` epoch counters (DEOPT_THRASH_THRESHOLD=64,
backoff `threshold << epoch` max shift 6, pin DISABLED after 8 epochs —
study abi.rs:5354-5400 analogue). True state-materialization deopt is OUT
OF SCOPE (study problem #3: "needs redesign; riskiest item" — file as
future work only if tier-1-only functions or inlining are ever wanted;
02.06 explicitly ships tier-0-equivalent binaries so nothing downstream
needs it).

**Files:** CREATE crates/ap-codegen-native/src/deopt.rs; modify
crates/ap-codegen-native/src/optimizing.rs. 

**Risks:** thrash backoff interacting with the background driver (02.05e) —
state machine TIER0/QUEUED/TIER1/DEFERRED/DISABLED lands here or in 02.05e,
pick one owner (recommend: states live in deopt.rs/tierup.rs shared module,
reviewed together).

**Gate:** deliberate-thrash probe (function flapping between int/str) ends
DISABLED with tier-0 results still byte-identical; corpus green adaptive.

**Deps:** 02.05c.

---

### 02.05e — Background tier-up driver + install discipline

**Scope:** tierup.rs: TierUpDriver (one background compiler thread, mpsc
CompileRequest), snapshot-at-enqueue (never concurrent reads of live cells —
feedback_snapshot on the enqueueing thread), pin-until-retire, install by
Release store + `QUEUED→TIER1` CAS, failure → retire_queued_to_tier0 /
TIER_STATE_DISABLED, shutdown joins compiler before dropping retained
modules. `APX_SYNC_TIERUP=1` debug mode. Adapted from study tierup.rs
discipline — pon's pin/snapshot/CAS model is explicitly right for no-GIL
(study "Background compile threading model": adaptable), and our GIL v1 does
NOT serialize the compiler thread (it only guards execution).

**Files:** CREATE crates/ap-codegen-native/src/tierup.rs; modify
crates/ap-codegen-native/src/lib.rs, crates/ap-codegen-native/src/runtime.rs.

**Risks:** the driver is the split's main concurrency surface — its tests
must be loop-tolerant (CI timing); retention of compiled modules vs GC.

**Gate:** PON_SYNC_TIERUP-analogous sync mode passes full corpus; async mode
passes a race-stress probe (tier-up mid-loop under mutation) 100/100;
failed-tier-up-disables test (pon has the exact test — port it).

**Deps:** 02.05c. Can be developed in parallel with 02.05d (interface: both
consume OptimizingPlan; only lib.rs wiring overlaps — sequence merges, not
work).

---

### 02.05f — OSR completion (boxed) + typed-OSR stretch

**Scope:** Emit the back-edge poll + OsrTransferBuffer transfer in baseline
codegen (osr.rs plan already landed: buffer layout, eligibility, liveness
canonical order — locals in LocalId order then live-ins ascending; >16 live =
ineligible, reported not silent). OSR entry compiles the reachable-from-header
CFG subset; OSR is a CALL not a frame transplant (tier-0 frame abandoned;
study control.rs:283-289). Typed-region OSR (unboxed live-ins at transfer)
is the STUDY'S OPEN PROBLEM #2 — keep it OUT of this sub-leaf's gate; box at
transfer instead (pon's own answer), file typed-OSR as a stretch follow-up
only if a corpus loop profile shows re-entry churn.

**Files:** crates/ap-codegen-native/src/{osr.rs,clif.rs,cfg.rs (backward
liveness dataflow over cfg::Func)}.

**Risks:** liveness dataflow correctness (the canonical slot order must match
between transfer emit and entry load — one shared constant table, tests
assert both directions); OSR frames stay on conservative-scan path (never
GC-map-registered — carry pon's rule).

**Gate:** hot-loop tier-up probe: loop crosses threshold mid-execution and
completes byte-identical; ineligible-loop probe (>16 live) still correct
purely via poll-based tier-up at function entry; corpus green.

**Deps:** 02.05c (OSR entry of a typed function needs regions) + 02.05e
(install path). Baseline-only OSR (boxed function, entry to baseline twin)
could technically land with 02.05a but buys nothing until tier-1 exists —
dispatch after 02.05c/e.

---

### 02.05g — GC roots shim: precise stack maps + fp-chain walker (Risk 1)

**Scope:** ap-gc/src/rootscan.rs: return-address-keyed SafepointMap registry
+ frame-pointer-chain walker (preserve_frame_pointers discipline in
codegen prologues) as the root-reporting shim handed to MMTk per
docs/gc-mmtk-swap.md §4; declare_gc_values analogue in clif.rs (mark every
pointer-typed value needs_stack_map; over-approximate i64s, filter by
span-table); scrub discipline (zero dead slots); conservative fallback +
conservative-scan-floor API for `del x; gc.collect()` observability.
Current state (PROGRESS-0205 Risk-1): boxed baseline keeps every value in
the ExecCtx slot array, which IS the shadow-stack-first choice — this
sub-leaf replaces/augments it with per-callsite maps only where tier-1 makes
it necessary. **Pairs with 02.09's GC safepoint work — see ordering.**

**Files:** CREATE crates/ap-gc/src/rootscan.rs; modify crates/ap-gc/src/lib.rs,
crates/ap-run/src/gcsupport.rs, crates/ap-codegen-native/src/clif.rs.

**Risks:** study problem #1 — "the hardest, needs real design work regardless
of pon" (MMTk binding contract redesign). The mitigating fact: tier-0-native
already has a correct precise frame (ExecCtx slots), so this shim is
incremental, not blocking. fp-chain requires frame-pointer retention —
measure the codegen perf cost before committing.

**Gate:** gc family 20/20 under adaptive; alloc-under-recursion probe
(fenced, per the harness hardening rules) forced-GC during deep native
recursion; `del x; gc.collect()` observability probe; corpus green.

**Deps:** 02.05a. Independent of 02.05b–f (can dispatch in parallel any time
after 02.05a). Shared-heap root reporting is ALSO the 02.09 pairing item —
coordinate, do not duplicate.

---

### 02.05h — Bench gate (T12): >=1.5x geomean

**Scope:** tests/bench/tier1/ (arithmetic + string slice), docs/tier1-bench.md,
measure adaptive vs forced-tier-0 geomean; record in PROGRESS.md. This is the
ORCHESTRATOR §02.05 exit gate for the whole split — only meaningful after
02.05c/d/e/f all land.

**Gate:** >=1.5x geomean over tier-0 on the arithmetic/string slice, corpus
green adaptive (the "tier-1 invisible except by timing" rule).

**Deps:** 02.05c+02.05d+02.05e (02.05f optional for the gate; OSR helps the
loop-heavy slice only).

---

## Dependency DAG

```
02.05a (entry hook + transparency)
  ├──> 02.05b (counters + ICs)
  │      └──> 02.05c (typed regions / tier-1 core)
  │             ├──> 02.05d (deopt)
  │             ├──> 02.05e (tier-up driver)   [parallel with 02.05d]
  │             └──> 02.05f (OSR completion)   [after 02.05e]
  │                    └────────────┐
  └──> 02.05g (GC roots shim)  ─────┤
        (parallel anytime)          v
                             02.05h (bench gate >=1.5x)
```

Recommended dispatch order (one heavy cargo lane — serial unless orchestrator
says the sibling lane is free):

1. **02.05a** — unblocks everything, zero-risk plumbing, substrate landed.
2. **02.05g** (parallel-eligible) — hardest risk in the study, starts earliest,
   and its output is shared with 02.09's GC work.
3. **02.05b** — after 02.09's object-state audit has claimed its classes.rs
   ownership window (see below), or sequenced immediately after it.
4. **02.05c** — the core; largest leaf, budget 2 waves.
5. **02.05d + 02.05e** (merge order d→e or e→d; independent code, shared
   state-machine review).
6. **02.05f** — OSR completion.
7. **02.05h** — bench gate; closes §02.05.

---

## 02.09 (no-GIL) ordering recommendation

**Recommendation: dispatch 02.09 FIRST — before 02.05c and everything
downstream of it. 02.05a may land before or alongside; 02.05b should follow
02.09's audit, not precede it. No 02.05 sub-leaf gates the 02.09 flip.**

Reasoning, grounded:

1. **T121 does not measure JIT code today.** The gate test
   (tests/t10_threading/test_t10_threading.py:423,
   `test_scaling_gate_3x_on_4cores_antipythony`, xfail strict=False) drives
   the ap-run interpreter via APX_IMPL — the JIT is not wired into
   `run_module` (that is 02.05a). Tier-1 perf is therefore orthogonal to
   whether the 3x scaling gate can go green: the flip is meaningful the
   moment GIL v1 stops serializing the interpreter, with or without tier-1.
2. **The orchestrator dispatch note already forbids re-gating**: "02.09 is
   dispatchable INDEPENDENT of 02.05/02.06 … do not let 'after tier-1' prose
   re-gate it". This split honors that; nothing above adds a 02.05
   precondition to 02.09.
3. **The real tradeoff runs the OTHER way.** "No-GIL without tier-1 may
   regress single-thread" is a real but TEMPORARY cost: per-object
   synchronization (atomics replacing RefCell discipline) taxes the
   interpreter's hot path until tier-1 recovers it. But building tier-1
   FIRST on a GIL'd runtime costs MORE: (a) the IC substrate is already
   seqlock/CAS-based (pon design, study: "exactly right for a no-GIL
   runtime") — under GIL v1 its atomicity is untestable, so 02.05b/c would
   need a correctness re-audit after the flip anyway; (b) 02.05b's version
   counters and 02.09's object-state audit mutate the SAME classes.rs paths
   — doing 02.09's audit first means the counters land on already-documented
   Send/Sync types instead of triggering a second ownership pass; (c)
   02.05g's root shim is literally 02.09's listed GC pairing item
   ("pairs with 02.05 Cranelift user stack maps") — one build, two consumers.
4. **Risk asymmetry.** 02.09's failure mode is a measurable single-thread
   regression (recoverable by 02.05c later). 02.05-first's failure mode is
   tier-1 built on GIL-serialized assumptions (IC torn-read paths, counter
   bump atomicity, driver thread model) that silently break at the flip —
   correctness-shaped, exactly the class of bug the IC audit exists to
   prevent.

Sequenced concretely: 02.09 → (02.05a ∥ 02.05g) → 02.05b → 02.05c →
02.05d/e → 02.05f → 02.05h. If 02.09 stalls (e.g. the audit uncovers a deep
ap-obj refactor), 02.05a+02.05g+02.05b may proceed — only 02.05c (typed
codegen) should wait for the flip, since that is where hot-path perf tuning
would otherwise be done twice.

## 02.06 AoT prerequisites

From the split, leaf 02.06 (leaves/02.06-aot-binary.md) needs:

| Prereq | Source | Status |
|---|---|---|
| cfg → CLIF lowering stages (cranelift-object consumes the SAME stages; AoT ships tier-0-equivalent code — adaptive tier-1 in the binary is an explicit NON-GOAL per the leaf doc) | 02.05 landed work (5d5c9e4/ecc097e) | **DONE** — 02.06's core dependency is already green |
| Feedback-slot discipline at final lowering (AoT compiles with a compile-time static snapshot; no runtime cells on the no-feedback path — but the SlotPlan ordering must match so JIT-vs-binary outputs diff clean) | clif.rs SlotPlan | **DONE** (re-lowering determinism test green per PROGRESS-0205) |
| ap-run init/run split + root feeds (generated entry point must call gc init, install root dicts/module-ns/sys.modules hook, catch_unwind per pon aot_entry pattern) | **02.05a** (entry hook work forces the init/run split in ap-run/src/lib.rs) + **02.05g** (root shim is what the entry installs) | REQUIRED — dispatch 02.06 only after these two |
| Tier transparency harness (`--tier 0` mode + differential runner) — 02.06's gate diffs forced-tier-0 JIT output vs binary output (same lowering ⇒ any divergence is a codegen bug) | **02.05a** | REQUIRED |
| ap-conform `--impl '<binary>'` mode | 02.06's own T5 (pattern exists: main.rs impl-string seam) | 02.06-local |
| importer reachable-module graph export (LTO dead-strip of vendored_stdlib) | 02.06's own T2 (read-only API on importer.rs — coordinate with whatever campaign owns importer.rs at dispatch time; it is a hot file) | 02.06-local |
| gc init/heap-sizing contract | docs/gc-mmtk-swap.md §6 (stable doc, no 02.05 dep) | none |

**NOT prerequisites:** 02.05b (ICs — binary path is no-feedback), 02.05c/d/e/f
(tier-1 core, deopt, driver, OSR — none ship in the binary), 02.05h (bench).
**Net: 02.06 can dispatch after 02.05a + 02.05g land — it does NOT wait for
the tier-1 core.** This is consistent with the orchestrator dependency line
"02.05 -> 02.06" read as "after the native baseline + seams", not "after all
eight sub-leaves". If the orchestrator wants 02.06 earlier still, the minimal
pruning is: extract the init/run split from 02.05a into a micro-leaf and ship
02.06 with the ExecCtx shadow-stack as the interim root story (documented
slower, acceptable for the binary's tier-0-equivalent code).

## Machine-load note (binding)

Every sub-leaf gate runs per-crate (`cargo test -p ap-codegen-native`,
fenced corpus probes) under the ONE-heavy-cargo-lane policy
(docs/test-interpreter-lock-policy.md): CARGO_BUILD_JOBS=4, workspace gates
only when the lane is free. 02.05e's race-stress probes and 02.05g's GC
probes are the noisy ones — schedule them when no sibling build lane is
active.
