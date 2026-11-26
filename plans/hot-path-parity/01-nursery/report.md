# 01-gc-nursery — report (leaf hp-nursery)

Worktree: `/Users/caimlas/.git/worktrees/leaf-hp-nursery` (branch
`tier1-hp-nursery`, forked from main @ `28480a0`). Machine: M2 Ultra,
release builds, quiet window, best-of-N per probe conventions.
CPython ref: /opt/homebrew/bin/python3.14 (3.14.7).

## Commits

| commit | what |
|---|---|
| `78d29ed` | perf(gc): thread-local registration ledger + dying-young flush shortcut + A/B/C probe modes |
| `a5e2b65` | fix(gc): three ledger re-entrancy/deadlock bugs + push_tracked ledger-edge arm + test-seam flush |
| `2373b6a` | test(gc): gc_invariants rides the flush boundary (assertions unchanged) |

## What landed (and what it actually is)

The leaf spec asked for a bump-arena nursery with per-slab registration
and handle-table promotion surgery. Measurement first (T1 bisect) showed
the cost model differently, and the design was adapted — per the spec's
"adapt as measurement dictates":

- `Gc<T> = Arc<T>`; the "arena" is the **thread-local registration
  ledger** (`PENDING`, one Vec push per mint, ~3ns) instead of the
  3-map registry critical section (~220ns) + ladder probe (~120ns)
  per mint. Batch flush at 2048 rows runs the SAME critical section
  + ladder the old per-mint path ran — allocation-count cadence
  preserved (`ALLOC_SINCE` counts rows at flush).
- **Promotion = flush**: rows enter the registry (young set) at the
  batch boundary or the next collect (`collect_gen` flushes first).
  No handle-table surgery: `Gc<T>` handles are already stable
  (Arc payload addresses never recycle while tracked), so the spec's
  "rewrite handles on promotion" problem does not exist in this
  architecture. The FALLBACK (promote on any external store) is
  effectively what the design does — an object still referenced at
  flush time promotes; one dead at flush never registers.
- **Dying-young shortcut** (the nursery's real win): a List/Dict/Set/
  Frozen row whose only live reference at flush time is the ledger row
  (`strong_count == 1`) is skipped entirely — no registry insert, no
  young-set entry, no pass ever scans it. Restricted to plain
  containers (no `__del__` surface, not weakrefable as builtins), so
  finalizer/weakref/resurrection observables are untouched. Obj
  instances always register (gc001/gc002/gc009/gc014 exact).

## Measured numbers (before -> after, same probes)

Raw mint microbench (`ap_gc_eval`, 160k 3-elem Smi lists):

| probe | main (`target-bisect42`) | leaf (this tree) |
|---|---|---|
| A keep (live) | 1097-1288ns | **1095-1195ns** |
| A drop (die young) | 1119-1193ns | **756-899ns** |
| A keep2 (warm) | 739-742ns | **804-849ns** |
| B insert-only (gc disabled) | ~220-290ns | **202-235ns** |
| C alloc-only (no hook) | ~48-56ns | ~48-56ns |

Interpreter-level (Python loops; includes tier-0 walk floor ~4µs/elem
that dominates every loop row):

| row | main | leaf | note |
|---|---|---|---|
| dict-mint 16k drop (native shape) | 5728-5838ns | **4545-4752ns** (-19%) | dict rows: dict.build 32k 201->194-205ns/elem (parity, not a win — walk-bound) |
| list mint 160k drop | 4000-4160ns | 4003-4220ns | parity (walk-bound) |
| json.loads 48KiB | 35.2 MB/s | 33.3 MB/s | parity |
| json.loads 1MiB | 31.1 MB/s | **32.5 MB/s** | slight win, inside noise |
| dict.get 32k | 524-538ns/elem | 524-537ns/elem | parity |

## Honest assessment vs the 985ns -> <=100ns target

**The target is NOT met, and cannot be met by registration batching
alone.** The premise decomposition (985ns = insert 160-250ns +
collect 400-700ns) was measured on the OLD tree before Fix-C landed
t0 700->7000 and the gen-1 retain fix. On this tree the same
decomposition measures: alloc 48ns + registration ~200-290ns
(insert-only B probe) + amortized collect ~500-900ns — and the B/C
probes prove the collect share is now the walk over LIVE young
objects at ~25ns/object steady-state, which batching cannot reduce
(it is the subtract-refs algorithm's inherent per-object cost: 3
hash probes + Arc::strong_count per scanned object per pass).

Three structural floors remain, each owned by a different lever:

1. **Registration ~200ns** (B probe): the 3-map critical section
   (registry + seq_of + young) under one process-global Mutex.
   Batching already amortized the ladder; the remaining cost is the
   3 inserts + Mutex themselves. Next lever: collapse seq_of into
   the registry row (1 map instead of 3) or shard REG_STATE.
2. **Live-object scan ~25ns/object/pass**: inherent to subtract-refs
   over Arc. Reducing pass frequency helps linearly but changes
   finalizer-timing observables the corpus deliberately does not
   pin (C-23/D1) — t0 is already 10x CPython's default (documented
   divergence). Not pursued further here.
3. **Interpreter walk ~4µs/step**: dominates every Python-level
   loop row (dict.build/get rows are 100% walk-bound — the mint
   share is <5% of the row). Owned by 03-jit-for-loops / tier-1,
   per master.md's dispatch order.

What IS solid and landed: correctness of the ledger through the
full gate battery (see below), the dying-young fast path (-23% on
die-young raw mints, -19% on dict-mint interpreter shape), and the
three re-entrancy/deadlock bugs fixed en route (one of them — the
mid-collect ladder deadlock — was caught by the corpus gc014 stress
hanging at 10 minutes and sample-proven before the fix).

## Gates (all run on the final tree)

| gate | result |
|---|---|
| cargo test -p ap-gc --release | 27 lib + 9 invariant, 0 failed |
| cargo test -p ap-obj --release | 0 failed |
| cargo test -p ap-run --release | 208 passed, 0 failed (all suites) |
| gc_differential + ast_getattr_identity_fence | 2/2 GREEN |
| fix_json (byte-exact) | 127/127, 0 mismatches |
| fix_int (byte-exact) | 9508 lines identical vs CPython |
| fix_sort | 42/42 |
| fix_random | PASS |
| diff_b64 | 2132/0 |
| diff_hex | 1976/0 |
| fix_sha_zlib | 84 diff lines, byte-identical to MAIN's output (pre-existing zlib divergences, verified not-mine) |
| ap-conform values | 50/50 |
| ap-conform stdlib | 48/48 |
| ap-conform gc corpus | 19/20 (gc018 FAIL — pre-existing: leaf output byte-identical to main binary; reported as expected-file drift) |

## Bugs found and fixed on the way (the load-bearing part)

1. **RefCell double-borrow** (gc_register -> flush -> collect ->
   flush_all_pending re-entry): batch detaches inside the closure;
   borrow ends before flush_pending runs.
2. **`FLUSHING.replace(false)` gate self-cancellation**: the idiom
   cleared the owner's flag mid-flight and classified the outer
   flush as the re-entrant inner — every flush skipped registration
   (registry empty; gc_differential caught it as gc-cycle-count
   returning 0). Gate is now plain `get()`; `flush_pending` is the
   sole owner of the flag.
3. **Mid-collect ladder self-deadlock** (gc014 hang, 10-min timeout,
   >=1569 mints): a `__del__` mint inside a running pass batch-
   flushed with suppress=true; the rows were suppressed but the
   LADDER still ran — its collect_gen blocked on COLLECT held by the
   outer pass. `sample` proof: collect_gen -> bridge_del -> attr_get
   -> PyObject::new -> gc_register -> flush_pending -> collect_gen
   -> COLLECT.lock wait. Fix: suppressed flush returns before the
   ladder; rows ledger on for the next collect.
4. **Ledger-resident edges invisible to the walk**: a fresh cycle
   whose member rows were still pending read sc=3/internal=0 and
   seeded reachable — never reclaimed (gc-cycle-count RED).
   `push_tracked` now counts ledger-resident children.

## Honest gaps

1. **<=>100ns amortized mint: NOT met** (see assessment; 756ns best
   die-young, ~200ns insert-only floor, walk-bound interpreter rows).
   The premise numbers came from the pre-Fix-C tree; the register +
   collect shares have different shapes now.
2. **json.loads rows: parity, not the hoped improvement** — the mint
   share of json.loads is smaller than the fix-c report's estimate
   once t0=7000 removed the collect tax; the residual is the parser
   kernel + per-Value boxing, not registration.
3. **dict.build/get rows: parity** — 100% tier-0-walk-bound; mint
   <5% of the row. Owned by 03-jit-for-loops per master.md.
4. **gc018 corpus FAIL is pre-existing** (leaf output byte-identical
   to main's binary; expected-file drift from an earlier merge).
   Not touched (corpus only grows).
5. **t0 default 7000 stays** (Fix-C's documented divergence); the
   ledger preserves its cadence semantics (rows counted at flush).

## Files touched

- `crates/ap-gc/src/gcmod.rs` (the ledger, flush, shortcut, observers,
  walk edge arm, gates)
- `crates/ap-gc/src/lib.rs` (flush_test export)
- `crates/ap-gc/tests/gc_invariants.rs` (flush-boundary seam)
- `crates/ap-cli/src/bin/ap_gc_eval.rs` (A/B/C probe modes)
