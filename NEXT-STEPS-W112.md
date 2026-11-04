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

- pluggy shim (generator-throw through native bound method): subagent
  (3.6h, 585 calls) narrowed to a deterministic ~30-line repro
  (/tmp/w112gen_y144.py + y145.py). KEY FINDING: the trigger is
  co_filename — frame files under staged site-packages (_pytest/*,
  pluggy/*) fail; identical source at /tmp/repo/other paths passes.
  Implicates importer.rs per-module state (fn registries / IR tables
  keyed by path) in the generator-throw relay. Eliminated: _multicall
  code itself, results[0], list-subscript arms, tb_frames drain,
  pending-gen-error slot. Full note + 3-step next-session plan:
  docs/w112-gen-throw-rootcause.md. Env-gated probes left in tree
  (APX_IDX_DEBUG / APX_GEN_DEBUG / APX_TB_DEBUG, zero-cost unset).
  Fix NOT landed; shim still required (noshim run = INTERNALERROR rc=3).
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

## Staged-tree pristine restore (this session)

Found stale probe scaffolding in staged _pytest/tmpdir.py (_probe_os,
tmppath_probe, guid_probe — earlier session's instruments, violating the
pristine-staging rule). Restored from the uv-cache pristine copy
(~/.cache/uv/archive-v0/2bu6F7ArekeHcOc9/_pytest/tmpdir.py), __pycache__
cleared, both cache spellings synced. T2 gate re-verified green after
restore (1F/1P/1S rc=1, zero INTERNALERROR). Before touching staged
site-packages: diff against the uv-cache copy first.

## Round-4 update (2026-11-04, commits d328a11 + f6ca150)

Gen-throw traceback campaign state (full detail:
docs/w112-gen-throw-rootcause.md):

- Commit d328a11: tb carry-back across instance re-raise
  (exception_value_of_instance / exception_instance / raise_from_value
  instance arm + __traceback__-chain walk fallback).
- Commit f6ca150: legacy Exception instances now accept `__ap_*` attr
  writes and READ instance attrs (both were silent no-ops before);
  sys.exc_info tags the materialized instance with __ap_tb__; RR path
  merges with in-flight EV tb (longer wins).
- Gates: T2 green with shim (1F/1P/1S, MAIN_RC=1, 0 INTERNALERROR);
  ap-run green except pre-existing vendored_types_and_abc_import
  (verified failing at pristine HEAD via git stash).

LAST HOP (isolated, single concrete step next session):
exception_instance never fires for the AssertionError because it
crosses pluggy teardown via teardown.throw (generator-resume fiber
path), not the Python `except BaseException as exc` catch. Evidence:
probe12 (plain chain) prints [APTB]; y144 (hook chain) does not.
Net: excinfo.traceback = 5 pluggy-only frames, all
__tracebackhide__ -> TracebackFilter empties -> code.py:1170
`traceback[-1]` IndexError. CPython's chain includes the USER test
frame; ours never records fiber-crossed impl frames.
NEXT STEP: instrument resume_live GenInput::Throw
(crates/ap-run/src/interp.rs:7483, also 4512/7711) to run the
frame-insert + exception_instance logic at the fiber-crossing catch,
exactly like exec_try's handler does (interp.rs:1510). Alternative:
append impl/test frames to exc.tb at the fiber crossing.

Other campaigns unchanged: scrapy crawl blocked on Deferred<->Future
bridge (deferred_from_coro repro) + engine._download:509 unpack error.

## ENVIRONMENT REGRESSION (end of round-5 session, UNRESOLVED)

After commit 562bb75 was verified GREEN (t2g8: 1F/1P/1S, MAIN_RC=1,
0 INTERNALERROR; probe15: longrepr renders 244-char traceback), a
shim-removal experiment broke the T2 gate and it has NOT recovered:

- Symptom: T2 driver rc=3, "1 passed", 16 INTERNALERROR ending in
  TypeError: 'object' object doesn't support item deletion with the
  last Python frame at pluggy _multicall:194 (tb misattributed; the
  real site is a __delitem__ on a legacy 'object'-kind instance
  during pytest collection - likely config.stash[key] deletion,
  staged stash.py differs from upstream).
- Verified NOT the cause: repo source (pristine 562bb75 stash-build
  also fails), code.py content (variant swap + byte-identical
  restore both fail), site-packages .pyc files (runtime has no pyc
  model - "v0 has no pyc model", never read), .pytest_cache
  deletion (g21 diverged further: 'type' has no attribute
  '_rawentry' from unraisableexception hook).
- Environment mutations made during the experiment (all reverted
  except cache deletions): code.py swap/restore in
  ~/.cache/antipythony/site-packages/_pytest/_code/, deletion of ALL
  __pycache__ dirs in BOTH cache trees + /tmp/w110_t2gate's
  .pytest_cache + __pycache__. NOTE: y144/probe15 repros now ALSO
  fail at perform_collect with the same del-item TypeError - the
  environment changed under the working repros.
- NEXT SESSION STEP 1: bisect the del-item TypeError. Lead: staged
  stash.py __delitem__ / config.stash[key] deletion during
  perform_collect; test a runtime __delitem__ dispatch on
  UserInstance/'object' instances (methods.rs subscript_del or
  equivalent). The antipythANY cache tree (driver sys.path[0])
  may also hold divergent files - diff antipythANY vs antipythONY
  for pluggy/_pytest before touching the runtime.
- Shim status: still REQUIRED (pristine FormattedExcinfo run rc=3),
  but the shim-blocker itself (IndexError on empty filtered tb) is
  FIXED at the source; dropping the shim now fails on OTHER gaps.

## Round-5 addendum (same session, after 100edc2)

Probe26 (decisive): the tb chain for trial A now contains TEN
entries INCLUDING the user frame (/tmp/...py) and the real assert
site (capture.py:860/861) - the fallback-__ap_tb__ fix works; the
chain is no longer pluggy-only. str(tbobj) renders without crash.

Remaining defect for the shim repr path: tb line numbers are
off-by-one low for several frames (369 vs 370, 193 vs 194, 145 vs
146, 860 vs 861). Root: raise-line attribution uses frame.last_line
which lags one statement marker under nested-if/try constructs
(probe27: raise at line 5, tb shows line 3). The shim formatter
indexes source lines with the shifted number and hits a None
lookup -> TypeError('NoneType' object is not subscriptable) inside
the makereport repr. Gate errors vary ('object' item deletion /
NoneType / _rawentry) because different cache/flow paths surface
the same underlying misattribution at different points.

NEXT SESSION STEP (DONE same session): lower_block Line markers +
synthetic-code co_* attrs fixed (commit 9f598e3). probe26 now shows
TEN tb entries including user frames + capture.py; str(tbobj)
renders; probe24 makereport outcome 'failed'.

## Round-5 final state (commits 9f598e3, 5fc99fd)

FIXED this round:
- lower_block Line markers (tb linenos now exact; probe29/30: f:5,
  g:7 vs CPython).
- build_tb_chain synthetic code: co_varnames/co_argcount/
  co_kwonlyargcount/co_flags - the shim repr crash root cause
  (inspect.getargs read None attrs). probe32 repr_failure now
  returns ExceptionChainRepr.
- T2 gate session RUNS TO COMPLETION with correct verdicts
  (test_pass PASSED, test_fail fails as expected), zero
  INTERNALERROR, rc=1.

LAST BLOCKER (driver crash after pytest.main returns):
collect_unraisable -> pop_unraisable() -> AttributeError 'type'
object has no attribute '_rawentry'. Committed WIP diagnostics in
5fc99fd (unraisable hook now passes the exception instance + real
tb chain; del-item __delitem__ dispatch widened) - crash persists.
Note the tb frames in the driver crash are OUR synthetic chain with
known misattribution, so the shown line numbers (46/55) may not be
the true raise site. Also: probe35's del-item TypeError at
pytest_collection_finish does NOT route through
methods::subscript_del (APDEL/APDEL2 instruments silent) - find the
actual del site first (suspect: a native-path deletion in the
collection hooks, or the error text originating from a re-raise of
a stale pending error).

NEXT SESSION ORDER:
1. Diff antipythANY vs antipythONY staged trees for pluggy/_pytest
   (driver sys.path[0] shadows one of them).
2. Fix the unraisable meta-object attribute chain (UnraisableMeta
   processing in staged unraisableexception.py vs what our hook
   provides).
3. Locate the real del site for the collection-finish TypeError.
4. Re-run T2 gate: expect 1F/1P/1S MAIN_RC=1 - then attempt the
   shim removal again (the original shim blocker is dead).

## Round-6 addendum (2026-11-04 late)

BREAKTHROUGH on the environment regression: the two staged trees
differ ONLY in _pytest/_code/code.py. The antipythANY variant (the
one the driver ACTUALLY loads via sys.path[0]) carries a T2
fallback in get_python_framesummary: when
extract_tb(self._rawentry, limit=1) raises IndexError (synthetic tb
objects can't feed the C-level extract), it builds the
FrameSummary from the frame's own attrs. The antipythONY tree was
missing this fallback - the current gate failures were this file
divergence all along. Both trees are now SYNCED to the variant with
the fallback.

Still failing: driver crash after pytest.main returns -
AttributeError 'type' object has no attribute '_rawentry' raised in
collect_thread_exception:46 / collect_unraisable:55 frames (our tb
attribution). APRAW instrumentation added at classes.rs:2841
(attr-get failure path for interned types) - does NOT fire, so the
failing lookup bypasses that arm. Note _rawentry is accessed ONLY
through pytest TracebackEntry instances (__slots__); receiver being
a 'type' suggests our runtime mis-binds self for __slots__-class
methods (self = the class instead of the instance) somewhere in
that flow.

NEXT SESSION (updated after round-6b instrumentation):
- APRAW-I (interp Type-arm) DID fire: receiver is literally the
  builtin `type`; pyloc = the driver trap frame (tb unreliable in
  this flow - sibling collector frames merged into one chain).
- NEW FACT: sys.unraisablehook is None in the script view (probe43)
  and APUNR never fired in the full flow - report_unraisable reads a
  runtime-internal sys namespace that differs from the Python-visible
  one pytest mutates. Verify the plugin collector can ever be invoked;
  fix the namespace bridge first.
- _rawentry is read only via pytest TracebackEntry instances
  (__slots__). Receiver `type` => either mis-bound self for
  __slots__-class methods, or an instance whose attr lookup fell
  through to its class (a type object). Check how our runtime binds
  self for __slots__-defined classes.
- probe35 del-item TypeError at collection_finish remains unlocated
  (does not route through subscript_del).

## ROUND-6 CORRECTION (critical mental-model fix)

The "ExceptionInfoFormatter shim" NEVER EXISTED as a staged
modification: pytest 9.1.1 upstream RENAMED FormattedExcinfo to
ExceptionInfoFormatter. The uv pristine code.py (58426 bytes) and
both staged copies are byte-identical and contain 4
ExceptionInfoFormatter refs. All prior "staged variant / shim
removal" reasoning was chasing a phantom - we have ALWAYS run the
upstream formatter, and the extraction fallback in the antipythANY
copy is the only real local divergence (now synced to both trees).

Current single T2 blocker: the unraisable/thread-exception collector
crash (AttributeError 'type' object has no attribute '_rawentry')
after pytest.main returns. Everything else in the T2 flow works.

## Round-7 addendum (2026-11-04)

- Namespace bridge VERIFIED WORKING: sys.unraisablehook set from
  Python IS invoked by report_unraisable (probe44/45: hook called,
  exc_type/exc_value readable). Added err_msg=None to the unraisable
  args object (CPython contract; pytest reads unraisable.err_msg
  first - it was missing and raised AttributeError before).
- Shim theory fully retired: with pristine upstream code.py in both
  trees the gate behaves identically - we always ran upstream
  pytest 9.1.1.
- Confirmed the failing read: `_rawentry` accessed on a receiver
  whose display name is literally 'type' (APRAW-I with
  LAST_PY_LOC). TracebackEntry is a __slots__ class; probe39 proves
  slots round-trip works for entries built on the main flow. The
  crash happens when entries are built INSIDE hook-wrapper fibers
  (sessionfinish teardown). Working theory: slots attr storage for
  instances created inside fiber-driven code is dropped or keyed
  differently, so the read falls through to the class (rendered as
  'type' in the AttributeError).
- Also confirmed: the crash is NOT logging-related (-p no:logging
  still fails) and NOT shim-related (pristine code.py identical
  behavior).

NEXT SESSION: test slots-instance attr storage inside a fiber:
create a __slots__ class, instantiate it inside a generator wrapper
resumed via pluggy-style teardown, read the slot. If it fails,
fix the slots attr store/read path in classes.rs (suspect per-fiber
attr isolation or weakref keying on the instance). Then re-run the
T2 gate.
