# NEXT-STEPS — W112 session (continues HANDOFF-2026-11-03.md)

## What this session (2026-11-03 evening) fixed — ALL COMMITTED
(absorbed into the rename-campaign commits 6c2c512/fc926d9 when that
agent committed the dirty tree; verified present in HEAD files)

Each fix isolated with a minimal probe in /tmp/w112_*.py, verified
side-by-side against python3.14 where applicable:

1. heapq/user-ordering: `py_lt` (builtins_modules.rs) now dispatches user
   `__lt__`/reflected `__gt__` via new `user_lt_public` (interp.rs) before
   falling to values_cmp. unblocks asyncio call_at heap (TimerHandle.__lt__).
   Probe: /tmp/w112_heap6.py.
2. asend_handle call arm (interp.rs ~6162): Namespace "asend_handle"
   called as a plain function drives the agen fiber; yield -> the result,
   exhaustion -> StopAsyncIteration.
3. `anext(agen)` builtin (builtins.rs ~4294): returns the __anext__
   AWAITABLE (CPython async_generator_asend shape) instead of driving the
   sync next() path ("'object' object is not an iterator").
4. eval_await agen-drive (interp.rs ~7520): when the await target is an
   asend_handle, the async-gen's yield IS the await's value (no relay out
   of the awaiting frame); exhaustion raises StopAsyncIteration.
   Probes: /tmp/w112_agen17/21/22.py — CPython-identical.
5. weakref on `__slots__`-declaring-`__weakref__` classes (gcsupport.rs
   weakrefable()): checks the MRO slots for "__weakref__" — scrapy
   Request/trackref WeakKeyDictionary unblocked. Probe: /tmp/w112_weakref6.py
   (CPython agrees: slotted-without-weakref NOT refable; sub OK; declaring OK).
6. threading._register_atexit name + call arm (threading_call.rs,
   vendored threading.py): twisted/concurrent.futures exit-hook surface.
7. importer parent-link `try_borrow_mut` (importer.rs ~1006): re-entrant
   import during parent-link write panicked "RefCell already borrowed"
   (crawl-killing panic; now skip-on-contended, idempotent write).
8. Stale probe line `_w84(...)` removed from staged scrapy engine.py
   (site-packages is test-data-only).

## Verified state at session end

- T2 gate (driver /tmp/w110_t2drv_trap.py, pluggy shim still in):
  1 failed / 1 passed / 1 skipped, rc=1, zero INTERNALERROR.
  CPython-identical. NOTE: run the FRESH binary via a COPY
  (cp target/release/antipythony /tmp/bin && /tmp/bin ...) — running
  target/release/antipythony directly served a stale binary during the
  concurrent rename-campaign build (cost ~30 min of phantom-bug debugging;
  see "stale-binary trap" lesson below).
- install_reactor hang: GONE (fixed by W111 changes). Full
  CrawlerProcess init + is_asyncio_available()==True works.
  Probe: /tmp/w112_r4b.py.
- Crawl (canonical process.start() path, /tmp/w112_crawl2.py + inline
  run_in_thread shim): spider opens, start() runs, request flows through
  ALL middleware to Downloader._wait_for_download. finish_reason=finished.

## THE NEXT BLOCKER (reduced to ~20-line repro)

`await deferred_from_coro(coro)` HANGS when the awaited coroutine itself
awaits another deferred_from_coro/future chain driven by the loop.
Minimal repro: /tmp/w112_dfc2.py

    async def inner(): await asyncio.sleep(0.05); return "P"
    async def outer(): v = await deferred_from_coro(inner()); return v
    @inlineCallbacks
    def top():
        result = yield deferred_from_coro(outer())   # HANGS HERE
    ...

Deferred.fromFuture(f) fires, but the awaiting coroutine never resumes —
the Task/future wakeup relay between Deferred.fromFuture and the
asyncio Task driving `outer` loses the wakeup. Same family as the old
"Task got bad yield" but at the Future.__await__ resume step. Fix in
interp.rs Task/future wakeup (vendored tasks.py __wakeup or the native
future-resume bridge). First action next session: instrument
futures.py Future.__await__ yield + Task.__wakeup for this repro.

## Crawl next-blocker refinement (added late this session)

The "too many values to unpack (expected 2)" is NOT the deferred_from_coro
wakeup relay: the full engine chain (inlineCallbacks nests, send_catch_log
arity=2, maybe_deferred_to_future, deferred_from_coro nesting) all pass in
isolated probes (/tmp/w112_nest.py, /tmp/w112_nest3.py, /tmp/w112_sig.py,
/tmp/w112_unpack3.py). The ValueError is thrown INTO engine._download /
_process_start_next as a Failure and its true origin frame is masked by
Failure.throwExceptionIntoGenerator. First diagnostic next session: run
the crawl with APX_TBDBG=1 or patch scrapy's Failure.raiseException
temporarily to print the FULL stored tb list, revealing the real origin
frame of the unpack. The deferred_from_coro repro (/tmp/w112_dfc2.py)
remains a REAL separate hang (await of dfc'd coroutine inside a
run_until_complete Task loop loses the wakeup) — keep both on the list.

## Also open (unchanged from prior handoffs)

- pluggy shim (generator-throw through native bound method): subagent ran
  3.6h, built first TRUE minimal repro (/tmp/w112gen_y144/145.py — prior
  synthetic repros were invalid), root cause narrowed; fix NOT landed.
- remote_control ext: aiohttp.web AppRunner missing (shim or disable ext).
- inline run_in_thread shim needed for the crawl (real threads lane):
  asyncio.to_thread -> loop.run_in_executor -> concurrent.futures
  ThreadPoolExecutor.submit fails at `self._counter()` ("'iterator' object
  is not callable" — itertools.count().__next__ bound-method surface).

## LESSON — stale-binary trap (record for future sessions)

When a parallel agent builds in the same target/ dir, executing
target/release/<bin> can serve a STALE binary (build ID race), making
freshly-compiled debug prints invisible and old bugs reappear. ALWAYS run
via `cp target/release/<bin> /tmp/bin && /tmp/bin ...` during multi-agent
sessions.

## Workspace gate verdict at session end

`cargo test --workspace --release`: 409 passed / 0 real failures. Nine
suites report FAILED inside the big workspace run (xtask notices x4,
packaging_differential x2, harness_selfcheck x2, vendored_types_import x1)
but EVERY ONE passes green standalone (verified per-suite immediately
after). Failure mode: spawned-binary + shared ~/.cache staging races under
parallel cargo test — the documented fixture/target-dir race class from
2026-10-27, now aggravated by the rename (both cache spellings exist).
Pre-existing flakiness, NOT a W112 regression. Follow-up: give the notices
gate and packaging tests isolated CARGO_TARGET_DIR / cache env.

## Cache-dir note (rename C3 side effect)

Runtime default cache moved to ~/.cache/antipythony (new spelling); the
staged pytest/scrapy tree still lives at ~/.cache/antipythany (old).
A copy of site-packages now exists at BOTH paths. Gate drivers insert the
OLD path explicitly and pass. If imports of pluggy/_pytest metadata fail
with PackageNotFoundError, refresh the copy:
  cp -R ~/.cache/antipythany/site-packages ~/.cache/antipythony/
