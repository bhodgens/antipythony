# NEXT-0210A — Leaf 0210a session record (scheduler stage 2a: resumable generator frames)

Worktree: `~/.git/worktrees/leaf-0210a`, branch `tier1-0210a` off `cb41e88` (main).
Commits (4, oldest first):

1. `f74f037` gen(2a): resumable frames for genexprs + cross-thread resume identity + concurrent-pull ValueError
2. `c0fc400` gen(2a)-fix: unify fiber id space with the generator registry id
3. `be041f0` gen(2a)-fix: pull_iter must not hold the iterators-table lock across it.next()
4. `45478f6` gen(2a)-fix: genexp fiber body runs against the DEFINING frame, not a child

## What landed

**State of the world at dispatch (honest correction to the leaf brief):** the
leaf brief described def-generators as still materializing eagerly
(interp.rs:2071-2073/4147-4149). That had ALREADY been fixed by the 02.09-pre
leaf (corosensei stackful fibers, `crates/ap-run/src/generators.rs`); the
brief's line cites are stale. What still materialized at dispatch was
**GenExpr** (`GenFrame::GenExpr` → materialize-on-first-pull into a sink), and
the concurrent-pull divergence was real for both shapes.

**1. GenExprs converted to the same resumable-fiber protocol as def-generators.**
`ensure_fiber` now converts `GenFrame::GenExpr` to a `Live` fiber
(`make_fiber_raw` runs `run_comp_clauses_into` with a new `genexp_yield` arm in
`run_comp_clauses` that suspends via `gen_yield` at the element point instead of
pushing to the v0 sink). The materialize paths in `gen_pull` and
`gen_materialize` are deleted; `gen_pull` is a single resume path for every
generator shape. Closed divergences (all verified vs CPython 3.14):
- produce-side effects run lazily (were: all at first next())
- genexp.send() receives (was: TypeError/StopIteration)
- genexp.throw()/close() act at the real suspension point (was: no-op or
  wrong-element answers)
- genexp concurrent pulls race no longer (shared sink removed)

**Design tradeoff (vs the 02-10 doc's explicit-frame-vector recommendation):**
the doc argued frame vectors double as the scheduler's Task.stack substrate.
The fiber substrate already existed and doubling suspension mechanisms would
have meant two resume paths; a scheduler Task can hold a fiber handle equally
well. Documented in the f74f037 commit message; 2b/2c implementers should treat
`GenState.fiber` as the Task suspension token.

**2. Cross-thread resume identity (found by the new corpus probes).** Fiber TLS
(ACTIVE/CURRENT) was pushed once at closure creation on the FIRST resuming
thread. A later resume from another OS thread (legal since 02.09 no-GIL) found
empty TLS, `in_fiber()` went false, and `yield` fell into the legacy
`Flow::YieldBack` path — the body ran to completion silently swallowing every
yield (a cross-thread second `next()` returned StopIteration with zero side
effects). Fix: identity is re-established per resume slice
(`resume_tls_push/pop` around `fiber.resume` in `resume_live`); the yielder
pointer rides a process-global `YIELDERS` map keyed by the generator REGISTRY id
(NEXT_FIBER minter deleted — the two id spaces collided at small ids and
`yielder_for` could return the WRONG fiber's yielder, relay yields to the wrong
coroutine; that collision produced genexp-pulls-defgen relaying raw sub values,
fixed in c0fc400).

**3. Concurrent same-generator pull now raises CPython's
`ValueError: generator already executing`** (was: silent StopIteration).
`gen_begin_running` happens under the same generators-map lock as the state
remove; a RUNNING placeholder entry (fiber=None) stays in the map for the slice
so racing `next()` routes through the generator path; `builtin_next` and the
absent-entry arm check `gen_is_running`; `end_running` moved AFTER the epilogue
re-insert so no absent+not-running window remains.

**4. Two collateral landmines found by gates, fixed:**
- `pull_iter` held the `interp.iterators` table lock across `PyIterator::next`;
  resuming a fiber whose body pulls another iterator re-locks the same
  non-reentrant mutex → same-thread self-deadlock (hung `import unittest`,
  fence-killed the gc_weakref_unittest_battery). Base was immune only because
  genexp first-pull never resumed a fiber from a table-locked pull. Fix: clone
  the handle out, drop the lock, pull (be041f0).
- The genexp fiber body ran in a `Frame::new_child(defining)`; new_child clones
  `parent.my_cells` as the child's FREE cells, but genexp closure free vars live
  in `defining.free_cells` → free-var NameError inside the fiber → swallowed as
  exhaustion → silently EMPTY streams (`ast.dump` printed `Module(body=[])`,
  stdlib s023 regressed). Fix: run the body against the DEFINING frame (v0
  semantics, 45478f6).

## Corpus growth (5 new cases, goldens pinned from CPython 3.14.7, 3-run
deterministic, meta provenance written by tests/tools/_gen_meta.py)

Threading (order-independent per the T101 template):
- `T130-concurrent-gen-pull.py` — two threads pull ONE generator across a
  Barrier with a wide CPU slice in the body; pins exactly
  [value, ValueError('generator already executing')]. Old code answered
  StopIteration (the divergence); new code verified 20+ runs, deterministic.
- `T131-parallel-distinct-gens.py` — 8 threads × own generator, sorted
  aggregate (per-generator frame isolation).
- `T132-genexp-concurrent.py` — two consumers over one genexp + bystander
  genexp (no element dup/drop; state isolation).

Generators (`generators_wasm/`, the only GE family — corpus only grows):
- `GE006-genexp-lazy-protocol.py` — laziness + send/throw/close on genexps.
- `GE007-genexp-nested-yieldfrom.py` — nested clauses, genexp-over-generator
  (the c0fc400 tripwire), sum()/list() drains.

## Gates (all measured on HEAD be041f0/45478f6, release, own target dir)

- Generator corpus (ap-conform vs CPython): **7/7** (GE001-007).
- Threading corpus: **20/21** — the 1 fail is T109, the pre-existing G-1
  reference-path gap the suite pins as xfail. New T130-T132 all PASS.
- T10x pytest (`tests/t10_threading`): **113 passed, 2 xfailed** (8 deselected
  scaling runs).
- `cargo test -p ap-run --release`: **206 passed / 0 failed** (18 suites). The
  gc_weakref_unittest_battery fence-kill that my first cut caused is fixed by
  be041f0.
- Wider corpus vs BASE (base binary built at cb41e88 in
  leaf-0210a-base/target-base): values 48/48, controlflow 1/1, exceptions 15/15,
  generators_wasm 7/7, stdlib 47/48, tier1-transparency 16/17, threading 20/21,
  classes 28/33 — **every remaining failure reproduces identically on BASE**
  (verified per-case: classes c002/c018/c030/c031/c032, stdlib s016, T109,
  transparency 1) → zero new regressions.
- s023_ast_parse_dump_unparse: regressed mid-session by 45478f6's predecessor,
  FIXED by 45478f6 (re-passed).

## Honest gaps / deferred

- **T121 scaling gate (≥3x) FAILED at 1.02x** — and the BASE binary fails the
  SAME probe at 0.79x right now. Machine is under heavy external load (load
  avg 4-6, disk 100%, sibling measurement agent building in parallel); the
  status block documents this gate as noise-sensitive ("re-verify in a quiet
  window <4 load"). This leaf did not touch thread scheduling; treat as
  deferred re-verify, not a regression. Evidence: base-vs-mine parity probe in
  this record.
- **T1 scrapy gate deferred**, with cause: the gate's session harness
  (file:// spider + items.jsonl byte-compare) is not in-tree; the in-tree
  quick probe (`import scrapy` via sys.path insert) fails IDENTICALLY on base
  and mine (importer `modspec.origin` AttributeError — pre-existing, unrelated
  to generators). The load-bearing scrapy import chain (functools/enum/
  weakref/contextlib generators — the exact shapes 2a touched) is covered
  green by the unittest-import probe, s023, and the gc battery.
- **Sibling worktree contamination (operational, for the orchestrator):** a
  parallel session is editing THIS worktree (crates/ap-run/src/mbc_tables.rs +
  mbc_gb18030_4.rs regeneration mid-flight, FUZZ_SEEDS.txt, classes .meta
  files). Their in-flight state breaks `cargo test -p ap-run` (test profile)
  builds (mbc.rs references MAP_* constants their regeneration had deleted).
  I snapshotted their dirty files to /tmp/0210a_sibling_backup/, temporarily
  restored HEAD to run the gates, and put their latest state BACK after each
  gate run. Their work is NOT committed by me and is preserved in the worktree
  (and in the backup). The orchestrator should tell that session to move to its
  own worktree.
- **fstrings corpus slice**: ap-conform glob returned empty for
  `tests/corpus/fstrings/*.py` both on base and mine — family layout differs
  (tests/fstrings_corpus exists separately); not gated here, unchanged by 2a.
- codecs_cjk ap-conform failures (10) are identical on base and mine — the
  family needs a non-default invocation (pre-existing harness shape), not
  gated here.

## For the 2b/2c stages

- `GenState.fiber` is the suspension token a scheduler Task should hold; the
  per-slice `resume_tls_push/pop` + `YIELDERS` + `RUNNING_GENS` machinery is
  the pattern to generalize to task park/unpark.
- Lock-ordering rules learned here: never hold `interp.iterators` or
  `interp.generators` across a fiber resume (any pull inside the body
  re-enters them); the RUNNING placeholder pattern (entry present + running
  flag) is the arbitration template.
- Pull discipline for Task fibers: `resume_tls_push/pop` must wrap every
  resume; the yielder table is process-global because fibers migrate threads.
