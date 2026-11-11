# Leaf 02.05d — Deopt thrash backoff (bounded, pon-analogue)

Status: LANDED on `tier1-0205d` (merged main at start: includes 02.05c
at e25fa20). Commit 7fb7337.

## What landed

- **deopt.rs** (CREATE) — the pon `abi.rs:5354-5400` analogue:
  - `TierState {Tier0, Queued, Tier1, Deferred, Disabled}` — OWNER per
    the SPLIT risk note ("states land here or in 02.05e"): 02.05e's
    driver consumes `TierState` / `tier_up_allowed` / `state_of` from
    here; the Queued state exists for it.
  - Packed per-function u64 word `state | count<<8 | epoch<<32` behind
    leak-once `AtomicU64` cells, keyed by the SAME def-pointer identity
    jit_hook's JIT_CACHE uses. `DEOPT_THRASH_THRESHOLD=64`, Deferred
    re-try threshold `threshold << epoch` capped at shift 6
    (`defer_threshold`), DISABLED pin after `DEOPT_PIN_EPOCH=8` epochs
    (final; further notes are no-ops).
  - `record_side_exit`: the state machine. Tier1 counts toward the
    epoch; the 64th exit drops to Deferred (epoch+1, count reset) and
    the 8th such drop pins Disabled. Tier0/Queued notes are tolerated
    no-ops (fail-open).
  - Concurrency posture: Relaxed orderings everywhere; a lost update
    can only cost one epoch of delay — fail-OPEN by design (advisory
    bookkeeping, never correctness). 02.09-safe; no GIL interaction.
  - Process counters `(deferred_drops, disabled_pins, side_exit_notes)`
    via `thrash_counters()`; `[TIER1-STATS] pin key=... epochs=8`
    machine-parseable probe line under APX_TIER1_STATS=1.
- **runtime.rs** — `h_deopt_note(ctx, fn_key)`: the ABI gains the fn
  key (clif `extra_params` table `&[]` → `&[0]`; helper_sig derives
  from the table, so no signature code changed). Delegates to
  `record_side_exit`.
- **clif.rs** — `compile_tier1(func, snapshots, fn_key)`:
  `emit_typed_fastpath` iconsts the key into the deopt tail so every
  side-exit edge notes per-function. On install success:
  `note_tier1_installed` (Tier1, fresh epoch — re-specialization after
  a Deferred drop starts clean). EN-ROUTE FIX (pre-existing latent
  bug): the thread-local SSA temp map in `emit_fastpath_body` was
  never cleared between functions — a second function's fast-path emit
  could read a stale temp from the first (silent WRONG VALUE). Now
  cleared at the top of every emit (`ssa_temps::clear()`).
- **jit_hook.rs (ap-run)** — two consumers:
  1. Tier-up probe gate: `deopt::tier_up_allowed(fn_key,
     jit_execs_now())` before try_tier_up — a Deferred function waits
     out `threshold << epoch` calls (JIT_EXECS as the monotonic
     progress signal) before re-tiering; Disabled never re-tiers.
  2. Dispatch gate (pon entry-pointer-restore analogue): the JitEntry
     caches the fn's deopt word at install; per call, a Disabled state
     bypasses the fast path and runs the boxed twin directly. The twin
     IS the same lowering tier-0 executes, so this is a pure de-cost
     with zero observable surface.

## State machine (final shape)

```
Tier0 --hotness gate--> Tier1 --64 side-exits--> Deferred(epoch k)
   ^                        ^  |                     |
   |                        |  +-- re-tier at threshold<<epoch    |
   |                        |      (capped shift 6)               |
   |                        +-------------------------------------+
   |                        (8 epochs observed) 
   +--- Disabled is FINAL; dispatch runs the twin (== tier-0 code)
```

## Gate evidence

- **Deliberate-thrash probe** `t1_017_thrash_backoff_disable.py` (17th
  transparency probe): `mix(a,b)` flips its binop site int↔str every
  100 calls × 60.1k calls → install line, then
  `[TIER1-STATS] pin key=... epochs=8` fires mid-run. Three-way exact:
  tier-0 == adaptive == CPython 3.14 (stdout `1809039900 / 12502500`,
  rc 0), post-pin steady-state int loop still exact through the twin.
- **tier_mode_differential 3/3**: mode-pair byte-identity over all 17
  probes (incl. t1_017), JIT-exercise table (compiles=8 JIT-ROUTED
  profile unchanged), and NEW gate 3 asserting install+pin observables
  on the thrash probe specifically.
- **deopt.rs unit tests**: 12 (state-machine defaults, exact pin
  landing at epoch 8 note #512, Deferred/backoff arithmetic incl.
  shift-6 cap, tier-up gating by call count, counters, Tier0 no-op
  tolerance). ap-codegen-native 44/0 total.
- **ap-run** all suites green (classes 29, method_surface 34,
  re 42+4 fuzz, import 6+9 guard, lxml 24, risk4 3, rootscan 4, gc 2,
  metaclasses 7, name_resolution 24, fix_pass 2 — the fix_pass ENOENT
  is the documented CARGO_TARGET_DIR artifact; resolves once
  ap-run-eval exists in the leaf target dir). ap-gc + ap-obj 101/0.
- **clippy -D clean** (ap-codegen-native, ap-run, ap-cli
  --all-targets), fmt clean.

## Deliberate divergences from pon (recorded)

- Deferred does NOT uninstall/restore an entry pointer per epoch; the
  fast path stays installed and the DISPATCH gate bypasses it only at
  DISABLED (Deferred's cost is the re-tier delay, not twin-only runs).
  Rationale: our twin == the tier-0 baseline compile, so "uninstall"
  is observably identical to "leave it and let guards fail"; the gate
  only fires where the win is guaranteed (thrash pinned).
- The pin is a function of EPOCHS OBSERVED, not compiles attempted
  (pon pins after 8 deferred epochs too, but ours counts even if
  tier-up never re-fires — simpler and strictly more conservative).
- Counter reset on pin (like pon); epoch number retained in the word
  for diagnostics.

## Open items (for 02.05e/f/h)

- `TierState::Queued` + `note_tier1_installed`'s Tier0/Queued arms are
  the 02.05e driver's insertion points (CAS install discipline lands
  there; this leaf's table is the shared state).
- The JitEntry `deopt_word` cache is set at install; 02.05e's driver
  must keep it in sync when it swaps retained modules.
- Bench impact of the per-call Disabled check (one relaxed byte load)
  is unmeasured; 02.05h measures the whole adaptive-vs-tier-0 delta.
