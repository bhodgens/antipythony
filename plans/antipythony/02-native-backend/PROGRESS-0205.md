# 02.05 native-codegen — leaf progress (worktree leaf-0205)

## Landed (commit chain on 6b8740c)

| Commit | Concern |
|---|---|
| 52c0c3c | LICENSES/MIT.txt + LICENSES/README.md (T10 — license obligation FIRST, before any pon-derived code) |
| 5d5c9e4 | cfg stage: structured ap-ir → cfg::Func + verifier rules (phase 2) |
| 7b084d8 | warning cleanup |
| b565e41 | clif+runtime: Cranelift boxed-Value baseline (phase 3, supersedes the b565e41..ecc097e range below) |
| ecc097e | clif.rs/runtimes finalized against cranelift 0.134 API + feedback tests |
| b0f6c81 | feedback substrate + IC audit + boxed-OSR plan + stack gate (phases 4+5) |

## Gates run (worktree, CARGO_TARGET_DIR=target-0205, CARGO_BUILD_JOBS=4)

- `cargo test -p ap-codegen-native`: 26 passed / 0 failed
  (7 lib + 7 cfg + 8 feedback + 4 osr).
- Tier-0 differential (tier-1-transparency family, tier-0 binary):
  19/20 probes + benches PASS byte-identical + exit-identical.
  2 fails total, BOTH PRE-EXISTING at trunk (verified on the untouched
  main checkout binary, same bytes):
  1. `t1_007_promotion_then_typeerror` — `round(float, ndigits)` with
     ndigits >= len(digits) mis-pads (root cause: `format_decimal` left-
     pads the digit string to `int_digits + ndigits` width, dividing by
     10^ndigits; `round(12570.0, 6)` → 1257.0). File:
     crates/ap-run/src/builtins.rs `format_decimal` — NOT in this leaf's
     file ownership (ap-run ownership is interp/methods/frame/gcsupport/
     lib only). Filed for the orchestrator: the fix belongs to a
     values-leaf or an explicitly granted ownership exception.
  2. `V016-encode-decode-errors` — utf-8 surrogate encode error missing
     (also reproduces byte-identical on main). Pre-existing, unrelated
     to codegen.
- gc (20/20) + values smoke slices PASS through the same binary.
- Stack gate: `worker_stack::tests::deep_chained_calls_survive_on_sized_stack`
  (Rust-level) green; corpus probe
  `tests/corpus/tier1-transparency/t1_016_deep_call_chain_500.py` green
  under the tier-0 binary (500-deep chained call, CPython-pinned triple).

## Risk-table status

- Risk 1 (GC roots shim): DECISION DOCUMENTED in cfg/clif docs — boxed
  baseline keeps every value in the ExecCtx slot array (a precise,
  GC-visible frame, MMTk-scan-able), which IS the shadow-stack-first
  choice of docs/gc-mmtk-swap.md §4 for native frames. The alloc-under-
  recursion probe (forced GC during deep native recursion) is wired only
  after ap-run routes execution through the JIT entry hook (T1 remainder).
- Risk 2 (typed-OSR): boxed-only per plan; eligibility cap
  (OSR_MAX_LIVE=16) reported not silent (osr_tests). Typed extension
  REMAINS OPEN (ordered sub-task).
- Risk 3 (true deopt): OUT OF SCOPE per leaf; cold-twin invariant holds
  (baseline compile is mandatory and unchanged by tier-1 later).
- Risk 4 (IC version audit): docs/ic-version-audit.md landed (18 rows,
  EnumDict + __ap_dict_*__ routes explicitly covered); audit_probe.rs
  (APX_IC_AUDIT=1 cross-check) landed with tests. ap-obj counter ADDITIONS
  (type/class-surgery versions) remain open with the tier-1 wave — the
  dict-version counter (PyDict::mutation_epoch) already exists and is
  pinned as the single source of truth.
- Risk 5 (slot stability): slots assigned ONLY in clif.rs
  (SlotPlan::assign); re-lowering determinism test green; no slot exists
  on ap-ir or mid-CFG (structural). Snapshot-carried slot-count wired
  into JitFunc.

## Open (honest remainder — partial landing per delivery contract)

- T1 remainder: ap-run entry hook (route run_module function bodies
  through the JIT when lowered) + ap-conform `--tier` flag plumbing
  (T11) — the differential corpus runs tier-0 today; forced-tier-0 vs
  adaptive byte-identity gate runs once adaptive exists.
- T5 tier-1 typed regions + guards + cold twin (optimizing.rs/region.rs/
  infer.rs) — substrate (feedback cells, SlotPlan, TypeTag) landed; the
  typed codegen itself is the next ordered wave.
- T7 tier-up driver (tierup.rs) — not started.
- T8 typed-OSR extension — boxed OSR plan + polls landed; entry transfer
  emission lands with the driver.
- T12 benchmark geomean measurement (>=1.5x gate) — requires tier-1.
- `round(float, nd)` pre-existing bug (above) — needs orchestrator
  ownership call.
