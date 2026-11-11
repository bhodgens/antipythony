# Leaf 02.05e — Background tier-up driver + install discipline

Status: LANDED on `tier1-0205e` (base = main incl. 02.05c wave 2). Branch
replaces 02.05c's synchronous tier-up at the call boundary with the
background driver (tierup.rs), per SPLIT-02.05 §02.05e.

## What landed

- **tierup.rs (CREATE)** — `TierUpDriver`: one background compiler thread
  (mpsc `CompileRequest`), the shared `TierUpState` machine (TIER0/QUEUED/
  TIER1/DEFERRED/DISABLED — this leaf is the OWNER per the split's
  Risk-4 recommendation; 02.05d's epoch backoff is the documented merge
  seam at the module level), and the pon discipline set:
  - **Snapshot-at-enqueue**: cells are snapshotted on the mutator thread
    (jit_hook) BEFORE enqueue; the compiler compiles from the frozen
    snapshot + a cloned `cfg::Func` (deterministic lowering, Risk-5) and
    never reads live cells.
  - **Pin-until-retire**: finished fast-path blobs are retained by the
    compiler thread for its whole lifetime; `shutdown()` JOINS the
    compiler before returning (pon shutdown discipline). Handed-out
    pointers stay valid regardless of drop order — cranelift-jit 0.134's
    `SystemMemoryProvider` leaks mapped code on drop BY DESIGN ("leak
    memory to guarantee validity of function pointers") and `free_memory`
    is never called anywhere in this codebase.
  - **Install by Release store + QUEUED→TIER1 CAS**: the compiler
    publishes into a per-driver completion map (`Ordering::Release`);
    a mutator's `poll(fn_id)` completes the install through exactly-one
    CAS. Racing mutators lose the CAS and run tier-0 one extra call.
    Completions are per-INSTANCE (process-global static was wrong under
    the parallel unit suite — fixed mid-leaf).
  - **Failure → DISABLED, never retried** (pon failed-tier-up-disables;
    the 02.05c `tier1_failed` rule moved into the shared machine; jit_hook
    mirrors it locally so the per-entry fast path stays a Cell load).
  - **APX_SYNC_TIERUP=1** debug mode: jit_hook keeps the 02.05c inline
    compile exactly, but the outcome (install/disable) routes through the
    driver's machine so stats/failure ownership has one home in both
    modes. Driver construction also exposes `start_sync()` for tests (the
    env var is read only by the real `start()` at ap-run init).
- **clif.rs** — the fast path's `JITModule` now lives IN `Tier1Func`
  (`_fast_blob`): the 02.05c `mem::forget` is retired and the "wrapping it
  requires moving the module into Tier1Func; that refactor lands with
  02.05e" note is discharged. Dropping a Tier1Func remains leak-safe by
  the provider design (documented at the emit site).
- **jit_hook.rs** — the sync site replaced: `tier_up_step` does
  poll → hotness-gate → enqueue (or sync-compile under the env flag);
  execution goes through `tierup::call_fast(fast_ptr, &je.jit, ctx)` whose
  status-2 hand-off falls back to the JitEntry's OWN baseline (the
  deterministic lowering makes the compiler's cfg clone slot-identical, so
  the fast path's guards line up with the cells already recording in
  flight — no twin blob ever crosses a thread). `JitEntry` retains the
  lowered cfg for the enqueue clone.
- **interp.rs** — process-exit seam: `shutdown_tierup_driver()` joins the
  compiler after `report_line()` (once per run_module; idempotent).
  `report_line` additionally prints `[TIERUP-STATS]` (state histogram +
  retained count) when the driver started and APX_TIER_STATS=1.

## Gate evidence (SPLIT-02.05 §02.05e gate matrix)

- **Sync mode passes the corpus (PON_SYNC_TIERUP analogue)**:
  `sync_tierup_mode_matches_async_across_corpus` — all 16
  tier1-transparency probes, `APX_SYNC_TIERUP=1` adaptive vs plain
  adaptive, stdout/stderr/rc byte-identical per probe.
- **Async race-stress 100/100**:
  `async_tierup_race_stress_100_runs_identical` — int→str-flip→int probe
  (t1_002 shape, racier phases), 100 runs under the async driver; every
  run's stdout + non-stats stderr identical to the reference schedule
  (only stats lines may differ — that IS the schedule-variance surface).
- **Failed-tier-up-disables (pon's exact test, ported)**:
  tierup_tests.rs `regionless_body_disables_and_never_requeues` (Ok(None)
  → DISABLED, repeat request rejected, never revives) +
  `failing_compile_disables_never_retries`-equivalent coverage via the
  state machine (compile Err → disable marker → DISABLED at poll).
- **Unit suite (tierup_tests.rs, 10 tests)**: wire round-trip +
  out-of-range→Disabled, exactly-once enqueue CAS, poll-install (first
  poll wins, later polls empty, retained count), frozen-snapshot compile,
  guard-backed-region cold compile contract, sync-mode fail-safe +
  note_installed/mark_disabled seams, shutdown-join + post-shutdown
  request fail-safe + idempotent shutdown, fast-pointer survival across
  pair drops, Tier1Func blob ownership (size sanity).
- **Regression**: ap-codegen-native 48/0 (38 pre-existing + 10 new),
  ap-run lib 4/0, tier_mode_differential 2/2 (mode-pair byte-identity +
  JIT-exercise), clippy `-D warnings --all-targets` clean on both touched
  crates, `cargo fmt --check` clean.

## Design notes / divergences (recorded)

- **Twin ownership**: pon ships the whole compiled pair across threads;
  our `Tier1Func`/`JitFunc` are NOT Send (they own `JITModule`s). The
  channel carries plain-data `FastPathHandle { fast_ptr, regions }`; the
  twin half is the mutator's existing baseline. This also removes the
  duplicate-twin compile+leak an earlier draft had.
- **The expensive half is fully async; the twin baseline stays
  synchronous** — it is the same compile 02.05a already did per function
  (~1-3ms debug, measured at the 02.05c gate), while infer+plan+typed
  emission (the part worth moving off the mutator) is backgrounded.
- **QUEUED is sticky until a mutator polls** — the compiler never mutates
  state slots (single-writer state machine, mutex-guarded map, CAS only
  on the Tier0→Queued and Queued→TIER1 edges).
- **02.05d merge seam**: `TierUpState::Deferred` exists and is documented
  as never produced by this leaf; whichever leaf lands second reconciles
  deopt.rs epoch counters into this module (thrash-DISABLED collapses
  onto compile-failure-DISABLED).

## Open items

- OSR (02.05f) will call `tierup::call_fast` for tier-1 OSR entries; the
  install path is already pointer-shaped for it.
- The `[TIERUP-STATS]` line currently prints only under APX_TIER_STATS=1
  alongside `[TIER-STATS]`; a dedicated APX_TIERUP_STATS knob is trivial
  if the 02.05h bench wants it separate.
