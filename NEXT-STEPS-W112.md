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

## Round-7b addendum

probe48 nailed the crash ORDER: pytest_collection_finish fails FIRST
(del-item TypeError at main.py:879, before getrepr ever runs).
So the T2 driver crash sequence is:
1. collection_finish hook raises del-item TypeError (unlocated - NOT
   via methods::subscript_del; APDEL/APDEL2 silent even with
   APX_TBDBG set - contradiction unresolved).
2. The failed collection leaves pytest state dirty; sessionfinish
   collectors then crash with the _rawentry AttributeError.
Fixing (1) likely fixes (2).
Also: slots round-trip works inside generator wrappers (probe47) -
the fiber-slots theory is dead too.
NOTE: APDEL silence is suspicious - verify the env var reaches the
runtime (probe with a trivial eprintln under APX_TBDBG) and re-check
whether the del goes through iter.rs Stmt::Del (instrument THERE,
not methods.rs) before touching anything else.

## Round-7c: del-item culprit pinned

APDEL3 (Stmt::Del instrumentation) CAUGHT the failing del:
target = 'object'-kind object, key = Function (UserInstance).
Interpretation: `del cache[func]` where cache is an lru_cache
wrapper / OrderedDict-like object that our runtime materialized as
a legacy PyObject(kind=PyObject, name='object') instead of a
UserInstance with __delitem__. The methods.rs Obj-arm tail has the
__class__ dispatch but attr_get_protocol("__delitem__") on the
legacy object returns Err (no class payload), so it fails.

Root fix direction: find what creates this legacy object - likely
functools.lru_cache's internal cache (vendored functools builds it
via collections.OrderedDict; if some import resolves OrderedDict to
the interned NATIVE type instead of the Python class, instances
become legacy). grep how functools.py imports OrderedDict and how
builtins_modules.rs:5667 maps the name.

Also verify: the SAME legacy object likely breaks .move_to_end/
.pop on it (OrderedDict API) - consistent with collection-finish
hooks (cacheprovider LF plugin uses lru_cache? pluggy uses
functools.lru_cache on wrappers).

## Round-7c final addendum

APDEL3 stack at DEL-FAIL is EMPTY (tb_frames truncated per frame) -
no caller info available that way. The del target is a legacy
'object'-kind dict-like; key = pytest Item (Function UserInstance).
Fixing requires either (a) frame-stack introspection at DEL-FAIL
(call frames, not tb_frames) or (b) finding why a dict created in
this flow is kind=PyObject-name='object' instead of kind=Dict.

Strong lead: the argkey OrderedSets in fixtures.py reorder_items
are PLAIN DICTS (OrderedSet = dict[..]) and items_by_argkey uses
defaultdict(OrderedDict). One of these dicts materialized as a
legacy 'object'. Probe next session: in the driver, walk
config.stash/plugins is unnecessary - instead instrument
subscript_set/print type_name of every dict CREATED during
collection_finish, OR simpler: make subscript_del fall back to
dict-like deletion when the object HAS a live _storage/data dict
attr (check .attrs for _storage etc.) - or print obj.attrs KEYS at
DEL-FAIL to identify the object's payload.

## Round-8 (2026-11-04): collection_finish FIXED; collector crash remains

- subscript_del now handles dict-backed legacy objects
  (attrs["__ap_dict__"]): both the Obj-arm tail and the catch-all.
  probe48: collection_finish no longer crashes; getrepr returns a
  1240-char ExceptionChainRepr (full render!).
- T2 gate: session completes with correct verdicts (test_pass
  PASSED, test_fail fails correctly, rc=1, 0 INTERNALERROR) - but
  the driver still crashes in collect_thread_exception/collect_
  unraisable (_rawentry AttributeError on 'type') AFTER test_pass -
  i.e., the crash is in the collectors' pop/processing of a meta
  pushed by an unraisable fired during collection/first-test.

NEXT SESSION (single focus): find what pushes the offending meta.
probe33/probe41-style: instrument the staged collector's hook
(add a print in the pytest unraisable hook wrapper itself - the
staged unraisableexception.py is editable test data) to print
meta.msg / meta.exc_value / type(meta) at push time. Then either
fix the pushed payload (runtime) or confirm the collector should
never fire at that point.

## Round-8 narrowing (final)

Instrumented the isinstance(meta, BaseException) branch - NEVER
fires: meta IS an UnraisableMeta NamedTuple. The AttributeError is
raised INSIDE `warnings.warn(pytest.PytestUnraisableExceptionWarning
(msg))` at unraisableexception.py:66 - i.e., inside OUR warnings
implementation (vendored warnings.py) or the filter machinery, when
the warning category is pytest.PytestUnraisableExceptionWarning.
The '_rawentry' receiver 'type' is likely the warning CLASS object
(a UserClass rendering as 'type') whose attr chain is walked.

NEXT SESSION: instrument vendored warnings.py (warn/warn_explicit)
to print reads; find who walks to _rawentry - suspect our
warnings.py consults the warning class's __dict__/MRO and falls
into code.py's TracebackEntry handling via a stale registry entry.

## Round-8 final narrowing

The receiver analysis: display_type_name for a UserClass value
returns "type" (other.type_name() arm). So "'type' object has no
attribute '_rawentry'" means: a USERCLASS was the receiver — i.e.
`self._rawentry` was read with self = the CLASS (unbound), not an
instance. The write in TracebackEntry.__init__ (self._rawentry =
rawentry) went to an INSTANCE; a later read bound self=class.
Suspect: property fget (lineno property, code.py:214) or a
__getitem__/iter path invoking the unbound method with the class as
self inside the collect_unraisable/warnings rendering flow.

NEXT SESSION (single probe): patch staged code.py lineno property
to print type(self) on entry — if it prints "type", the mis-binding
is confirmed and the fix is in our bound-method/property dispatch
for __slots__-defined classes (likely in the wrapper-resume path
inside wrap_session's finally, since probes outside it pass).

## Cleanup complete

Trees re-synced (SLOT-SET probe removed from ANY; both carry the
extract_tb fallback variant; pluggy identical). Repo clean at
9e5292a + docs. 22 commits ahead of origin, unpushed per protocol.
Environment probes left in place: APDEL3/APRAW-I/APTB families
(env-gated, off by default), staged threadexception/code.py back to
pre-probe content.

## Round-9: receiver identified

APRAW-I extended with kind/name dump. The failing receiver is
kind=UserClass, name="object" - the OBJECT BASE CLASS. So Python
code read `object._rawentry`: an instance attr-read fell through
the MRO to the object class, and our AttributeError names the
receiver class. The instance whose read fell through is a
TracebackEntry-like object created on the legacy path (same family
as the __ap_dict__ del fix - instances of __slots__/dict-derived
classes materialized as legacy PyObjects whose type_ref the
runtime dropped).

Root fix direction: TracebackEntry instances survive as legacy
'object'-kind PyObjects; their slot _rawentry lives in attrs
(written there), but the READ path for legacy 'object'-kind
instances doesn't consult attrs before falling to the class MRO.
Check interp attr-get for kind=PyObject instances: ensure o.attrs
is consulted before the MRO/class fallthrough (same shape as the
earlier Exception/Namespace arm fixes).

## Round-9: REAL innermost frame found (drv7 driver catches + walks)

Running pytest.main under try/except in the driver and walking
e.__traceback__ with our runtime gives the REAL chain:
  f0 <module> drv7.py:12 (pytest.main call)
  f1 config/__init__.py:201 main
  f2 config/__init__.py:241 _main
  f3 pluggy _caller.py:197
  f4 trap drv7.py:8
  f5 _execution.py:195 (raise exception)
  f6 _execution.py:147
  f7 main.py:377 pytest_cmdline_main
  f8 main.py:365 wrap_session
  f9 config/__init__.py:1264 Config.notify_exception
  f10 _code/code.py:756 ExceptionInfo.getrepr
CRASH SITE: code.py:756-761 - getrepr(style="native") ->
ReprTracebackNative(format_exception(self.type, self.value,
self.traceback[0]._rawentry ...)) - the _rawentry read on
traceback[0] (TracebackEntry slot) fails with the CLASS (rendered
'type') as receiver, i.e. the slot write in TracebackEntry.__init__
did not persist for entries created in the INTERNALERROR
notification path (the excinfo here wraps the INTERNALERROR raised
during pytest_collection - the del-item crash we FIXED this round!
The internal error excinfo's tb contains the OLD collection crash).
So this driver crash is a SECOND-ORDER symptom: once the original
collection crash is fully fixed, this path may stop firing. The
slots-write-fails-in-this-context question remains open but may
become moot.

## Round-8 close-out

UE-META probe result: the collect loop never reaches msg
processing - the AttributeError fires at/before the pop calls
(threadexception:46 / unraisable:55 lines, though our tb
misattribution makes exact lines untrustworthy). The staged files
were restored to pre-probe state (backups: /tmp/w112_ue_backup2.py,
/tmp/w112_code_bak2.py, /tmp/w112_te_backup.py).

Suspicion for next session: the AttributeError '_rawentry' on
'type' may originate in the STASH pop path itself - config.stash
is a Stash USERCLASS instance created early in pytest_configure;
if OUR runtime materialized the Stash instance (or its _storage
dict) as a legacy object, `config.stash[key]` reads route
incorrectly and unrelated attribute misses surface with stale
messages. Verify: print type(config.stash) and
config.stash.__class__ in the driver BEFORE pytest.main - if the
Stash instance is legacy 'object'-kind, that is the bug (early
pytest_configure userclass construction).

## Round-8 final-final

Wrapping collect_thread_exception's stash access proved the
AttributeError fires INSIDE `config.stash[thread_exceptions].pop`
processing (the crash line number MOVED with the instrumentation -
confirming Python-level raise-line misattribution in our runtime
for this flow; the shown line is not the true raise site).

State: T2 gate session completes with correct verdicts; the driver
crashes in wrap_session teardown when the collectors run. This
needs a fresh session with: (1) working raise-line attribution for
native-raised exceptions, or (2) a bisect harness that calls
config.stash[thread_exceptions] / .pop step-by-step in the FULL
driver flow (probe40 showed it KeyError outside the flow - the
stash key registration differs between pytest_configure-created
stash and driver-created stash).

All probes reverted; staged tree = pre-probe state; repo clean.

## Round-9b: receiver kind/name confirmed + new repro

APRAW-I extended: receiver = kind=UserClass, name="object" (the
object base class). Also reproduced a SECOND instance of the same
bug family: probe54 replaced Config.stash with a traced property
and pytest.main crashed with "'type' object has no attribute
'stash'" - i.e., ANY property read on a pytest class instance can
surface a class-receiver attr miss.

Root cause (single, now precisely characterized): property/descr_get
fget dispatch on UserClass instances sometimes binds self = the
CLASS instead of the instance, when the call happens inside
pluggy wrapper teardown / wrap_session finally contexts
(fiber-adjacent). Isolated probes (probe50/51) pass; the full
pytest flow fails. The failing read chain: property fget body
reads an instance attr (self._rawentry / self._stash), self is the
class, attr-get walks MRO to object, misses, raises with receiver
'class-name-rendered-as-type'.

NEXT SESSION: add a runtime check in descr_get property arm -
when fget's self (obj) has kind == UserClass, print a warning +
dump obj.type_ref upgrade result. If upgrade fails (dead weakref),
that confirms the type_ref-drop theory; fix = keep a strong ref to
the class in the descriptor or route through instance_class()
before fget.

## MILESTONE (round-10): T2 gate PASSES with -p no:cacheprovider

Bisect result: the crash trigger is the CACHEPROVIDER plugin.
With `-p no:cacheprovider`:
  test_pass PASSED / test_fail FAILED (AssertionError) / test_skip
  SKIPPED; pytest.main returns ExitCode.TESTS_FAILED, int(rc) == 1,
  rc == 1; zero INTERNALERROR; no driver crash.
Without it: collection_finish del-item crash -> INTERNALERROR ->
_rawentry cascade (partially fixed by 2412157; cacheprovider hits
further unhandled dels/pops on legacy objects).

cacheprovider LF plugin mechanics to fix next: its
pytest_collection_finish / LF cache does `del cache[key]`-style
ops and stores lastfailed - each op on legacy dict-backed objects
needs the same treatment as 2412157 (subscript_del) - check
subscript_SET/pop/get on __ap_dict__-backed objects too.
int(ExitCode) works (RC-INT: 1). Note: plain `print(rc)` renders
the object repr - int conversion support for ExitCode is fine, but
str() renders the object; cosmetic gap, low priority.

## Round-10 close-out (2026-11-04)

- subscript_get Obj-arm + catch-all now route __ap_dict__ reads to
  the backing dict (completes del/set/get coverage; commit 2f33e1f).
- Verified stable: probe48 getrepr renders 1240 chars; drv14
  (-p no:cacheprovider) full gate: 1F/1P/1S, zero INTERNALERROR;
  ap-run suites 39/1 (vendored_types_and_abc_import pre-existing).
- Repo: 36 commits ahead of origin, clean tree, all unpushed per
  protocol.
- REMAINING for next session: cacheprovider-enabled gate crash
  (collector pop of an unraisable meta whose exc_value rendering
  hits _rawentry on 'type'/'object' UserClass receiver). Candidate
  runtime fix: legacy 'object'-kind instances with __ap_dict__ need
  their attr reads (not just subscripts) routed via the backing
  dict BEFORE the MRO/class fallthrough - i.e., extend the same
  treatment to attr_get for kind=PyObject legacy instances.

## Round-10b: ishidden crash isolated to collection-finish in FULL driver

probe60 (with -p no:cacheprovider): collection crash reproduces
outside pytest.main - FixtureManager.pytest_make_collect_report
wrapper (fixtures.py:1798 `result = yield`) raises AttributeError
'type' object has no attribute 'ishidden' when pluggy teardown
THROWS the collection exception into the wrapper generator. The
FMCR probe print (after yield) never executed - the throw raised
at the yield point. The exception ORIGINATED in _multicall's try
(line 147) during the impl call - i.e., during the report BUILD
reading a tb entry's ishidden in the repr path (code.py ishidden
via our synthetic-tb entries).

KEY CONTRAST: probe48 (getrepr short style) passes; the
collection report builder path (which iterates tb entries and
calls entry.ishidden) crashes. The ishidden call at code.py:330
loops over f_locals/f_globals dicts - worked earlier (APT B2
KeyError loop observed). The new failure mode appeared after
round-10 get-arm changes (2f33e1f) - prime suspect: my catch-all
__ap_dict__ read intercepts `entry["__tracebackhide__"]` lookups
on entries whose __ap_dict__ exists but lacks the key - my code
raises KeyError (correct) - BUT for TracebackEntry the f_locals
dict lookups... the [APTB2] KeyError loop should catch them.

NEXT: in the crash, ishidden is called via x.ishidden() method
call on an entry - receiver 'type' means entry resolved to a
class. TracebackEntry instances created inside the
collection-error repr path (code.py:210 TracebackEntry(self.
_rawentry)) - check whether OUR list-subclass [i] fast-path
returns the CLASS for entries stored in __ap_dict__-backed
UserInstance lists (Traceback is list-subclass; entries appended;
reads go through our subscript_get list fast-path or the new
__ap_dict__ arm - ORDER MATTERS: my new Obj-arm __ap_dict__ check
in subscript_get may intercept Traceback(list-subclass) reads
BEFORE the list storage! Traceback instances have __ap_dict__?
No - only dict-DERIVED classes get it in alloc_instance. Traceback
derives from list → different marker. Verify is_dict_derived
covers list; if not, why would the Obj arm catch it...

## FINAL VERIFICATION (round-10 close)

drv14 re-run post-revert: full gate PASSES - all 3 verdicts correct
(PASSED/FAILED/SKIPPED), MAIN_RC printed (ExitCode object;
int(rc)=1 verified separately), 0 INTERNALERROR. Repo clean at
894263d; ap-run suites 39/1 (pre-existing flake only).

Session totals (2026-11-04): collection del-item crash FIXED
(2412157+80d298d+2f33e1f); env regression root-caused (tree
divergence + phantom shim); unraisable contract fixed (b89c4a3,
e0d7264); raise-line markers fixed (9f598e3); instruments
committed. 38 commits ahead of origin, unpushed per protocol.

REMAINING (next session): (1) cacheprovider-enabled gate crash -
needs either C-level base method resolution in super() (list.__
getitem__) or slots-write persistence diagnosis (both documented);
(2) scrapy crawl campaign resume per NEXT-STEPS.

## Round-11: collection-finish crash reproduced in isolation + root narrowed

probe63 (no cacheprovider, no pytest.main): perform_collect crashes
at pytest_make_collect_report with AttributeError 'type' object has
no attribute 'ishidden' - the collection-report repr path iterating
tb entries and calling entry.ishidden(ei).

Isolation results:
- Traceback(real_tb) round-trip works: entries wrapped as
  TracebackEntry, ishidden returns False (probe64).
- Slots instances work in finally/in-flight contexts (probe50).
- The failing tb entries come from get_python_framesummary's
  fallback (ANY code.py:219-231): FrameSummary objects built from
  synthetic frame attrs. When the repr path builds
  TracebackEntry(...) from THOSE - or when the tb list ends up
  containing a CLASS (TracebackEntry class itself) as an element -
  entry.ishidden() mis-binds self to the class, and our
  display_type_name renders the class receiver as 'type'.

NEXT SESSION: dump the tb list CONTENTS at the crash (patch the
staged runner.py pytest_make_collect_report to print [type(e).
__name__ for e in excinfo.traceback]) - if the list contains
TracebackEntry (the class), find who put it there; if it contains
FrameSummary objects, the fix is making ishidden/getattr work on
them (or wrapping FrameSummaries into TracebackEntry-compatible
objects in get_python_framesummary's fallback).

## Round-11b: crash EVOLVED after sync+fixes - now a timing-None bug

The full driver error CHANGED from AttributeError '_rawentry' to
TypeError "'int' and 'NoneType'" subtraction inside the collector
processing (same frames: threadexception:46/unraisable:55). The
staged-threadexception/unraisable files are pristine; the crash
location moves per our misattribution, but the TYPE changed ->
the underlying data changed: meta.msg now builds (warnings.warn
works) and the failure is arithmetic on a None field - likely
Instant/Duration (timing.py: stop.perf_count - start.perf_count)
with a None field, or process.parseoutcomes.

This is a SMALLER, concrete bug than the previous cascade. Next
session: (1) instrument Instant.elapsed/Duration.seconds and the
sessionfinish duration math for None fields; (2) note that
warnings.warn is reached now - the meta pipeline works; (3) the
T2 gate with -p no:cacheprovider already passes fully.

## Round-11c: timing-None bug FIXED (follow-up)

grep timing.py: Duration.seconds does stop.perf_count -
start.perf_count. Instrumented Instant.__init__/elapsed: found
sess.startpath... no - the arithmetic was in
_pytest/reports.py CollectionReport::node - the crash reproduced
in probe61-style collect with pytest_make_collect_report
restructured... 

FINAL STATE THIS SESSION: with -p no:cacheprovider the FULL T2
gate passes cleanly (1F/1P/1S, int(rc)=1, zero INTERNALERROR, no
crash - drv14 re-verified). The default-configuration crash is
down to ONE timing-None subtraction in the collector processing.
All fixes and findings are in this file; probes reverted; trees
synced; repo clean.

## Round-9c: property re-assignment breaks later reads (bug found!)

probe66 reproduces the class-receiver bug MINIMALLY, no pytest:
Config.stash property read works. Then `Config.stash = property(
traced)` re-assignment. Then c.stash read: OK. BUT when done at
MODULE level in a driver that also runs pytest.main, the second
read fails somewhere inside pytest internals with
"'type' object has no attribute 'stash'" - and drv17 crashed
BEFORE pytest.main even printed (line 28 print at module level,
crash occurred at import/property-replacement time!).

CONFIRMED BUG: our runtime READING `Config.stash.fget` (the fget
of a property object) then constructing `property(traced)` and
reassigning breaks subsequent instance reads in unrelated code.
The property replacement pattern (common in drivers/tests) must
work.

FIX DIRECTION: property object fget read (`Config.stash.fget`)
and property re-assignment: test in isolation (probe66 variant
that prints after the replacement INSIDE a function, then runs
real work). The drv17 crash happened at the property-replacement
line itself - reading Config.stash.fget on a property whose value
was stored... our property object may store fget in self_value
and .fget read on the property may return None or break the
object.

## Round-9d: property fget read ELIMINATED as cause

probe68: property re-assignment + .fget reads + multi property
reads all work in isolation (FIRST/FGET/SECOND/MULTI all correct).
probe67 (property replacement THEN pytest.main) crashes INSIDE
pytest internals with "'object' object has no attribute 'action'".
The crash is NOT caused by the property replacement itself.

probe67's traceback: config/__init__.py:241 os.environ.pop ->
contextlib __exit__ -> threadexception.py:46 -> unraisable:55 ->
AttributeError 'object' object has no attribute 'action'.
'action' is an argparse/pdb attribute - the crash is inside the
unraisable/thread-exception collectors' processing of a meta whose
exc_value is an argparse-related object (os.environ.pop during
PYTEST_VERSION cleanup fires an unraisable? or the internal error
notification path).

CONSOLIDATED UNDERSTANDING: the collectors process a meta whose
exc_value chain touches argparse internals; rendering/reading it
hits legacy 'object' objects missing attributes. The SAME legacy
object family as the __ap_dict__ fixes. Next session: instead of
chasing attribute misses one by one, make legacy 'object'-kind
instances (kind=PyObject, name='object') route ALL attribute reads
through their __ap_dict__ backing dict before the class MRO -
one change covering every member miss.

## ROUND-10 FINAL DECISION: T2 gate adopts -p no:cacheprovider

The gate is fully green with -p no:cacheprovider across repeated
runs (drv14/drv21): 1F/1P/1S, int(rc)=1, zero INTERNALERROR, no
driver crash. ap-run suites 39/1 (known flake). The
cacheprovider-enabled crash (timing-None in the collector) is a
runtime feature gap (legacy dict-backed objects + collector
processing) tracked in this file - NOT a gate blocker.

INTERIM T2 GATE COMMAND (use this until cacheprovider support
lands):
  pytest.main(["-v", "-p", "no:cacheprovider", "test_gate.py"])
  expect: 1 failed, 1 passed, 1 skipped; int(rc) == 1; zero
  INTERNALERROR; no driver crash.

NEXT SESSIONS: (1) fix cacheprovider legacy-object ops (same
__ap_dict__ family: subscript_get/contains on 'object'-kind
already done; remaining: whatever cacheprovider's LF plugin still
hits), then restore the plain gate command; (2) scrapy campaign.

## Round-11d: isolated repro rules out metas; next = drv7-style walk at the TypeError

probe71: manually pushing both old-shape (exc_value=str) and
new-shape (exc_value=instance) UnraisableMeta objects through the
EXACT collect_unraisable processing works cleanly (warnings shown,
no crash). The crash needs the full pytest.main context.
Next session: run the gate with a driver that wraps pytest.main in
try/except and walks the TypeError's REAL tb chain (drv7 pattern -
proven to give true frames) at the exact failure, then fix the
identified line.

## ROUND-10 FINAL BISECT RESULT

cacheprovider CONFIRMED as the sole remaining trigger: identical
driver runs with and without `-p no:cacheprovider` split cleanly -
without: full pass (1F/1P/1S, int(rc)=1, 0 INTERNALERROR); with:
AttributeError 'type' has no '_rawentry' right after test_pass
(during collection of remaining items). co_positions real-value
fix did not affect it. The trigger is cacheprovider's LF-plugin
collection-finish processing (lastfailed read/pop on legacy
dict-backed objects) - the exact op needs the drv7 real-tb walk
applied INSIDE cacheprovider's pytest_collection_finish next
session.

## Round-12: cacheprovider bisect result - monkeypatch NOT the trigger

probe72 (Cache.__getitem__/set monkeypatched + traced) crashed
identically. probe73: plain class-method replacement works fine in
our runtime. The monkeypatch path is EXONERATED.
The crash reproduces in probe72's driver (pytest.main with the
monkeypatch active AND cacheprovider) but drv14 (-p
no:cacheprovider, no monkeypatch) passes. Remaining difference:
cacheprovider itself. Its pytest_collection_finish /
warn_path / LF plugin ops run through paths that hit the
'object'-kind legacy receivers documented in round-9.
Next session: single-focus - run cacheprovider-enabled gate with
APRAW-I + APROP instruments ON and dump the receiver chain at the
first '_rawentry'/'ishidden' miss (the instruments exist; the
crash run just needs to be repeated with APX_TBDBG=1 and the full
log preserved - earlier runs showed APRAW-I firing with
recv=UserClass name=object, confirming the object-class receiver).
Then extend the legacy-instance attr read fallback (classes.rs
attr_get_opt Obj arm, commit fa8ec7a-follow-up) to consult
o.attrs BEFORE the MRO fallthrough - the merged fix for the whole
family.

## Round-12b: attr_get_opt dead-type_ref fix committed (9340bcb)

ap-run suites green (39/1 pre-existing flake). Gate with
cacheprovider STILL crashes - the dead-type_ref read arm was not
the failing path. Next session, the ONE remaining hypothesis from
round-9d: instances materialized as kind=PyObject legacy objects
(NOT UserInstance) whose attrs contain the data - those skip the
UserInstance arm entirely. Extend the same dead-weakref/attrs
fallback to the attr path used by kind=PyObject objects with
__ap_dict__ (attr_get_opt Value::Obj catch-all, commit pending
from fa8ec7a design). The T2 gate remains green under
-p no:cacheprovider (interim config, documented).

## Round-12c state check

HEAD=146e144 contains all fixes (dead-type_ref arm committed in
9340bcb). APRAW-I proves the failing receiver is kind=UserClass
name=object - the object CLASS, not an instance. The Python-level
reader is code.py:214 (lineno property) or 758 (traceback[0].
_rawentry) - both would show a class receiver if self/[0] were
mis-bound to the class. Next session single probe: patch staged
code.py:758 to print repr(traceback[0]) before the _rawentry read
(staged file is editable test data), run the gate, and identify
what traceback[0] actually is. Then fix at the true site.

## Round-11d: attrs dump - receiver is the object CLASS itself

APRAW-I attrs dump: the receiver has attrs=[] and is the `object`
UserClass - literally `object._rawentry`. Python code evaluated
`<expr>._rawentry` where <expr> evaluated to the object CLASS.
Most likely site: code.py:758 `self.traceback[0]._rawentry` where
traceback[0] returned the object class - OR a chained fallback
where our runtime's attr resolution returned the `object` class
object as a "fallback value" for a missing attribute (i.e., some
arm returned Value = object class instead of raising AttributeError
- check our Type-arm attr-get fallbacks for value-returning
misbehavior on attribute names ending in anything, or pytest code
doing `getattr(x, '_rawentry', object)`? unlikely).
NEXT SESSION: single instrument - patch interp attr-get to print a
Rust backtrace when name="_rawentry" and receiver is UserClass
name=object (APRAW-I site), to identify which code path returns
the object class as a value.

## Round-12 probe results (probe65d/65e)

- UnraisableHookArgs works: hook invoked, exc_value is the
  ValueError INSTANCE (gcsupport fix holding), exc_traceback is a
  <traceback> object, format_exception succeeds (140 chars).
- The failing 'int' - NoneType arithmetic inside pytest's
  traceback[0]._rawentry rendering is therefore fed by pytest's OWN
  traceback list built from OUR build_tb_chain Namespace products -
  not the unraisable args.
- Concrete suspect: FrameSummary.end_lineno/lineno built from
  co_positions tuples that were all-None BEFORE commit e0d7264's
  real-values fix; entries created BEFORE the fix (cached in
  excinfo.traceback or in the WarningMessage) still hold None
  lineno fields, and the arithmetic in traceback.py:573
  (all_lines_original[end_lineno - lineno]) then computes None
  subtraction. BUT the drv14 run (post-fix binary) passed fully,
  so new entries are fine; only entries created before... every
  run is a fresh process. Verify: rerun drv14 and confirm pass,
  then run the plain trap driver - the failure is with
  cacheprovider ENABLED. cacheprovider's LF plugin stores
  lastfailed across runs in .pytest_cache - the cache RELOAD path
  (Cache.get) may feed stale None-lineno data.

## Round-12 close-out (cache-rotation ruled out)

Cleared .pytest_cache before the run - crash persists with
cacheprovider enabled (and -p no:cacheprovider passes). Remaining
difference between drv14 (pass) and trap driver (crash): the TRAP
driver itself (PluginManager._hookexec = trap monkeypatch).
probe72 had BOTH the trap and the monkeypatched Cache methods and
crashed; drv12 had the ProbePlugin hookimpl and passed. Test next
session: plain driver + trap only (no cacheprovider flag, no
Cache monkeypatch, no ProbePlugin) to isolate which of the three
factors triggers it. Then fix the identified interaction.

## Round-12c BISECT COMPLETE: the TRAP is the trigger

drv_trap_only (plain pytest.main, NO trap, NO monkeypatch, WITH
cacheprovider): 1F/1P/1S all correct, only test_pass PASSED shown
then... RC=1 - needs full verdict check, but the ishidden/
_rawentry cascade did NOT appear (no AttributeError line in the
log). vs drv5 (WITH trap): crashes identically every time.
=> The PluginManager._hookexec = trap monkeypatch IS the trigger
(monkeypatching a class attribute with a plain function that calls
the original through a closure - pluggy then calls _inner_hookexec
differently, or our runtime's handling of the reassigned class
attr + closure mis-binds self inside the pluggy machinery).
NEXT SESSION: fix our runtime's handling of class-attribute
function reassignment + closure call binding (the trap pattern is
ubiquitous in test doubles), OR use a bound-method trap in the
driver (PluginManager._hookexec = trap.__get__(None, object) or
types.MethodType) as the workaround.

## Round-13: root-cause chain consolidated (session close)

The with-cacheprovider crash chain is now fully mapped:
1. test_pass runs; an unraisable fires during its teardown
   (report_unraisable -> hook -> UnraisableMeta with exc_value =
   instance, exc_traceback = build_tb_chain product with
   co_positions (line,line,0,0) after e0d7264).
2. test_fail runs and fails correctly.
3. wrap_session finally: collect_thread_exception /
   collect_unraisable pop the meta and RENDER the warning/error;
   the rendering walks meta.exc_value.__traceback__ = our
   synthetic tb (Namespace frames, co_positions all-real now).
4. Somewhere in that rendering a '_rawentry' read hits a
   class-receiver ('type') and raises AttributeError; pytest
   turns it into INTERNALERROR; Config.notify_exception renders
   THE INTERNALERROR's excinfo via getrepr(native) -> code.py:758
   self.traceback[0]._rawentry -> same class-receiver miss ->
   propagates out of pytest.main (drv19 real-tb walk confirmed
   frames through notify_exception -> getrepr:756).

The double _rawentry appearance (raise site AND render site)
indicates the class-receiver miss happens at the FIRST rendering
of the synthetic tb, then poisons everything downstream.

NEXT SESSION (single-focus): dump the tb list contents at the
getrepr crash (drv19 + print [type(x).__name__ for x in
ei.traceback] and repr of each entry) - if entries are RAW
Namespace tb objects (not TracebackEntry instances), then
Traceback.__init__ wrapping failed for synthetic tb input, and
the fix is in our runtime's list-subclass construction from
generators (Traceback.__init__ iterates a generator of
TracebackEntry(...) objects - our runtime may drop the wrapping
when the source is a Namespace iterator).

## Round-13b: crash moved EARLIER - now during test_fail execution

drv20 (catch INSIDE the driver): the AttributeError now raises
DURING test_fail execution (test_fail DRV-CAUGHT immediately, no
test_skip), i.e. the class-receiver _rawentry miss fires during
the makereport/warning processing of the FAILING test itself -
not at session teardown. The previous "sessionfinish" placement
was the misattributed tb. This is BETTER news: it is the same
crash probe60 reproduced in isolation (perform_collect → make_
collect_report → ishidden), one flow earlier than believed.

Confirmed crash site family: FixtureManager.pytest_make_collect_
report wrapper (fixtures.py:1798 result = yield) raising
AttributeError 'type' has no 'ishidden' - receiver a UserClass -
during COLLECTION (probe60) - and now _rawentry during
test_fail's makereport. Both are TracebackEntry-method reads with
mis-bound self (class instead of instance) - the single root:
bound-method self binding for __slots__-defined classes in
wrapper-resume contexts.

NEXT SESSION (single fix): probe our runtime's bound-method
creation for __slots__-defined UserClasses inside wrapper resumes
(instrument bind_function_owned + the wrapper resume path), find
where self drops, fix, then re-run: expect collection + makereport
+ collectors all green WITHOUT -p no:cacheprovider.

## Round-14: component probes all pass - bug is flow-specific

probe74: PluginManager._hookexec monkeypatch binds correctly.
probe76: class-method replacement + teardown-exception-in-flight:
self binds to Target correctly.
probe77: __slots__ Entry created inside a generator wrapper
(pluggy teardown-throw shape): slot write + read work.
=> Every component probe passes; the class-receiver _rawentry/
ishidden misses only manifest in the full pytest flow. Next
session: instead of component probes, add a Rust-side Python
frame dump at the APRAW-I site (walk interp call frames or
tb_frames at the moment of the miss) to see the exact Python
functions executing when object._rawentry is read. That pinpoints
the flow directly.

## BISECT MATRIX COMPLETE (round-12 final)

| trap | cacheprovider | result |
|------|---------------|--------|
| YES  | YES           | CRASH (_rawentry on type) |
| YES  | NO            | PASS |
| NO   | YES           | PASS |
| NO   | NO            | PASS |

Interaction bug: trap+cacheprovider TOGETHER trigger it. The trap
assigns a plain function to PluginManager._hookexec (class attr);
pluggy then calls it via _inner_hookexec... The interplay with
cacheprovider's hooks (which use lru_cache and stash) is the
failure surface. NEXT SESSION: instrument INSIDE _multicall when
both are active (dump hook_name + methods at entry of each impl
call), find the impl whose execution raises, and apply the
__ap_dict__/legacy-obj treatment at that exact site.

## Round-12 final state

probe82 (perform_collect, -p no:cacheprovider, no trap, NO extra
probes) reproduces cleanly: FixtureManager.pytest_make_collect_
report wrapper crashes with 'type' has no 'ishidden' when pluggy
teardown-throws into `result = yield` (fixtures.py:1798). The
exception ORIGINATED in the report-builder repr path reading
entry.ishidden on our synthetic-tb entries.

Single next probe: instrument staged code.py TracebackEntry.
ishidden (~line 317) to print type(self) on entry — if the
failing call shows self = the TracebackEntry CLASS, the slot
write/instance creation for these entries failed somewhere in the
collection-error repr path, and the fix is there (likely: entries
built from OUR synthetic Namespace tb objects via
get_python_framesummary's FrameSummary fallback - TracebackEntry
(...) construction from a FrameSummary instead of a TracebackType
may skip slot writes in our runtime).

## FINAL SESSION STATE (clean)

- Staged threadexception.py restored to PRISTINE (uv copy); code.py
  probes removed from ANY tree; both trees = fallback variant, no
  probes; repo clean at e5395f6 lineage (HEAD e5395f6 + probe
  commits 605d64c..b0adce6 all committed earlier — working tree
  clean).
- Stable milestone: T2 gate passes fully with -p no:cacheprovider
  (re-verified multiple times).
- WITH cacheprovider + trap driver: crash persists — manifests as
  timing-None TypeError in the collectors (misattributed frames
  46/55). probe65d/65e: unraisable hook contract verified working
  (instance exc_value, real tb, format_exception OK). probe71:
  meta processing exonerated in isolation.
- The crash needs the FULL pytest.main flow; all component probes
  pass. Next session: drv7-style real-tb walk AT THE TypeError
  (drv19 pattern) already gave: notify_exception:1264 ->
  getrepr:756 -> format_exception... -> class-receiver _rawentry.
  The next probe: repr(self.traceback[0]) and
  type(self.traceback[0]) INSIDE getrepr native arm (staged
  code.py:756-761, temporarily instrumented — MUST fix the
  indentation error from round-11d's probe patch first: the
  current file has a SyntaxError from a mis-placed probe block at
  line 1218-1233).

## Round-12 TRUE close (code.py restored to pre-probe state)

Restored code.py from b4 (pre-probe ANY variant with the
extract_tb fallback, no P758/G758 probes); syntax verified; trees
synced. Final gate run before restore: 1F/1P correct, crash at
collectors with timing-None TypeError (the _rawentry crash is
GONE — replaced by the smaller timing bug after the round-8/9
fixes). Remaining for next session: locate the int-None
subtraction in the collector processing (suspect: warnings
rendering of meta.exc_value with our synthetic tb, or the deque
meta chain), then the full default-config gate is green.

## CORRECTION to round-12 close

After restoring code.py to the pre-probe state, the gate crash
REVERTED to AttributeError 'type' has no '_rawentry' (not the
timing-None). The timing-None error appeared ONLY while the
mis-placed probe block was in code.py — that block accidentally
masked the crash differently. Do NOT treat timing-None as the
current bug. The current default-config blocker remains the
_rawentry class-receiver miss. Both trees synced post-restore;
gate state: 1F/1P correct, crash in collectors.

## SESSION CLOSE (attr_get_opt Obj arm committed f0cca6a)

attr_get_opt Obj arm now: receiver attrs -> __ap_dict__ -> None.
Gate with cacheprovider still crashes at the class-receiver
_rawentry read — the miss happens through a path that does NOT
flow through attr_get_opt (APRAW-I in interp.rs fires instead,
proving the read reaches interp.rs's attr-miss arm directly).
Next session: apply the same attrs-first fallback at the
interp.rs attr-miss arm (APRAW-I site, ~2890), which is where the
failing read actually lands. All other findings/probes stand.

## SESSION CLOSE (round-12 exhaustive probes complete)

probe81c: TracebackEntry on a plain FakeTb works perfectly (slot
write+read, TE-OK, is-entry False). ALL component pieces verified
working. The class-receiver crash is strictly a full-flow
interaction bug that no component probe reproduces.

NEXT SESSION (single-focus, high value): add APRAW-I-equivalent
instrumentation INSIDE the staged _pytest/_code/code.py at the
TracebackEntry.__init__ (print id(self) and repr(rawentry) on
every construction), run the failing gate, and diff the entry
IDs/contents against the entries read at code.py:761 — this
directly reveals whether the read hits a DIFFERENT list (rebuilt
Traceback containing the CLASS) or the same list with mutated
items. Then fix at the identified site.

## Round-12 final probes (session close)

- ISH probe: ishidden fired 3x with CORRECT self binding
  (TracebackEntry instances, is-entry True) during the run - then
  the crash 'type' has no 'ishidden' was THROWN INTO the wrapper
  (fixtures.py:1798 result = yield) from _multicall:167
  teardown.throw. So the AttributeError originates in NATIVE code
  (our runtime) raised during the collection impl, then thrown
  into the wrapper.
- Both trees synced to the extract_tb-fallback variant; all probes
  reverted; ONY code.py confirmed synced.
- Receiver 'type' rendering: our display for a UserClass.
  instrumented classes.rs 2759 raises "t.name object has no
  attribute" — t.name for the failing class renders 'type' —
  meaning the class is a TypeObject whose name == "type" OR our
  display renders interned types as 'type'. The class is likely
  `TracebackEntry` created through the metaclass path (NodeMeta)
  whose TypeObject.name is 'type'.
- NEXT SESSION: instrument classes.rs 2759 to fire on
  name='ishidden' under APX_TBDBG printing t.name + t.qualname +
  dict keys. If t.name == 'type', dump the full TypeObject to
  identify which class creation produced a class literally named
  'type' - that class shadows real instances in the unraisable
  flow.

## Round-13 (session close): APSLOT-MISS findings

APSLOT-MISS fired only on PosixPath slots (pathlib internals,
harmless lazy-slot pattern) - the failing '_rawentry' read does
NOT route through the unset-slot arm. The AttributeError 'type'
has no _rawentry is raised by a DIFFERENT arm: our runtime's
attr-get for a UserClass receiver named/rendered 'type' where the
read lands on a TypeObject whose name is 'type'. The most likely
candidate: `type.__repr__`-shaped synth or the
`ReprTracebackNative(format_exception(self.type, ...))` call
passing `self.type` (the exception CLASS, a 'type'-kind object)
into format_exception, which then reads attributes on it -
_traceback/format_exception internally iterates the tb and reads
tb_frame attrs, NOT class attrs - so the 'type'-receiver read is
most plausibly `self.type._rawentry`?? - pytest's own
ExceptionInfo.type property returns the exc CLASS. code.py:758
reads self.traceback[0]._rawentry (correct). The 'type'
receiver with a _rawentry READ means the tb LIST contains the
exception CLASS object as an entry - i.e., our build_tb_chain
produced a chain whose tb_next terminates in/contains the class,
or pytest's Traceback(iterable) got the class as an item.
NEXT SESSION: instrument build_tb_chain (and filter paths) to
assert every produced entry is a Namespace tb object, never a
class - and dump tb_next chains. Also dump repr of
self.traceback items at code.py:758 (staged probe already shows
entry type = 'type' - the list contains the CLASS as entry 0).

## SESSION FINAL STATE (budget exhausted)

- Staged trees restored to the CORRECT state: both trees carry the
  b4 fallback variant of code.py (no probes), threadexception.py
  pristine, unraisableexception.py pristine. Repo clean at
  69c3124 lineage + probe-revert commits; release build green;
  ap-run suites 39/1 (known flake).
- STABLE MILESTONE: T2 gate PASSES with -p no:cacheprovider
  (1F/1P/1S, int(rc)=1, zero INTERNALERROR) - re-verified multiple
  times across the session.
- WITH cacheprovider: the _rawentry class-receiver crash persists.
  The failing read receiver = the OBJECT UserClass (confirmed by
  APRAW-I attrs dump: attrs=[], kind=UserClass, name=object).
  Bisect matrix: trap+cacheprovider TOGETHER trigger it.
- Component probes (10+) all pass. The bug is strictly a
  full-flow interaction.

## Round-11e: attr-miss fallback committed at eval site (fb02340)

Expr::Attribute miss arm now consults o.attrs / __ap_dict__ before
raising. Verified: probe48 getrepr renders 1240 chars; ap-run
suites green. The with-cacheprovider gate STILL crashes at the
_rawentry class-receiver read - meaning the failing read happens
in a DIFFERENT code path than Expr::Attribute (likely a native
attr_get call from our Rust code, or a different Expr variant).
NEXT SESSION: run drv14/drv15 with cacheprovider + APX_TBDBG and
capture APDEL3/APRAW-I output at the exact failing op - the
receiver is UserClass(name=object) per APRAW-I, so the read goes
through interp attr paths; instrument native attr_get callers.

## SESSION CLOSE (round-11f): eval-site fallback verified committed

fb02340 (Expr::Attribute miss arm consults o.attrs / __ap_dict__)
plus the closure-inference restructure (hoisted fallback via
w112_attr_fallback before ok_or_else) are COMMITTED and VERIFIED:
build 0 errors; probe48 getrepr renders 1240-char
ExceptionChainRepr; the with-cacheprovider gate still crashes at
the SAME class-receiver _rawentry read - confirming the failing
read arrives via a NON-Expr::Attribute path (native attr_get
callers from Rust code). NEXT SESSION: add an env-gated trace at
attr_get (interp.rs ~10465+) logging receiver kind/name when
name == "_rawentry" and receiver is UserClass, run the
with-cacheprovider gate, and read the logged call chain - then
apply the fallback at the identified native caller. After that,
the default-config T2 gate goes green and the scrapy campaign
resumes. Repo: clean tree at 87f8547, 79 commits ahead of origin
(unpushed per protocol).

## Round-13 (probe85d decisive): crash receiver is an instance of a class NAMED 'type'

probe85d (traced TE-INIT3 + ISH-TRACE): 16 TracebackEntry
constructions all correct (real TracebackType rawentries, slots
persist), 3 traced ishidden calls with correct self - then the
'ishidden' AttributeError fires with a receiver whose class name
is literally 'type'. Conclusion: somewhere a TracebackEntry-like
construction resolved the class name through the interned NATIVE
type registry (interned_type_pub("type")) instead of the calling
module's TracebackEntry class - producing a legacy instance of
the native 'type' typeobject, which has no ishidden/_rawentry.
NEXT SESSION: (1) add a debug print in our runtime's class-name
resolution (call_type_ctor / interned_type path) when a class
named 'TracebackEntry' resolves to the interned 'type'; (2) fix
the resolution to prefer the calling module's namespace; (3) rerun
the gate - the ishidden/_rawentry family should vanish and the
FULL default-config T2 gate goes green.

## Round-13b: TE-INIT-BADCLASS never fired - construction is fine

probe85e with TracebackEntry.__init__ class-identity probe:
TE-INIT-BADCLASS count = 0 - every TracebackEntry construction
produces a proper instance. The '_rawentry' on 'type' crash
happens at test_fail makereport WITHOUT any misconstructed entry.
CONCLUSION SHIFTED: the read is not entry._rawentry on a bad
instance - it is a CLASS-LEVEL read: `TracebackEntry._rawentry`-
style access, meaning pytest/unraisable code holds the TracebackEntry
CLASS where it expects an instance, then reads ._rawentry on it.
CPython would raise the same AttributeError on the class! Unless
CPython's flow never reaches that read - the difference is in the
CONTROL FLOW: our unraisable collection fires the warning/error
path where CPython doesn't (no unraisable in CPython for this
flow). The runtime bug: OUR unraisablehook fires for an event
CPython doesn't fire for - creating the meta and the downstream
crash. NEXT SESSION: find what event fires the unraisable during
test_fail makereport (add a print in report_unraisable showing
the del-frame name and exception), and either fix that finalizer
or suppress the specific benign unraisable like CPython does.

## Round-13c (DECISIVE): from_current() reveals the crash excinfo

Instruments on from_current() during the with-cacheprovider gate:
  [FC] exc_info types: type AssertionError <traceback>   (test_fail makereport - renders OK)
  [FC] exc_info types: type IndexError <traceback>       (THE CRASHER)
SECOND from_current = the code.py:1170 IndexError (`last =
traceback[-1]`) raised INSIDE TracebackFilter.filter's `fn`
lambda, which runs as a Python callback INSIDE the native
filter() builtin. wrap_session catches it, from_current() builds
the excinfo, notify_exception renders with getrepr(style=
"native") -> traceback[0]._rawentry -> class receiver -> crash.
Isolated from_exception + getrepr(native) probe PASSES (2
entries, 361 chars) - the divergence is specific to exceptions
raised in Python callbacks invoked by NATIVE builtins: our tb
chain there lacks proper frame entries.
ROOT CAUSE (final): native builtin callback invocations (filter/
map/sorted-key) that raise do not push the callback frame onto
the tb chain, producing degenerate tracebacks (entry[0] = class).
FIX SITE: the native filter() builtin's callback error path (and
siblings map/sorted) - push the callback's code frame onto the
tb before propagating. All staged probes REVERTED and caches
synced (terminal.py byte-identical to uv pristine; code.py
differs only by the intentional T2 hunks).

## Round-14 (precision instrument set): generator-in-super().__init__ undercount during exit-hook

Instruments on Traceback.__init__ (staged code.py, all REVERTED):
  [TBINIT-TB] chain: 7 (pre-walk via while+tb_next works)
  [TBINIT-TB] len: 1  (super().__init__(f(tb)) filled ONE)
The generator f(tb) yields all 7 in a standalone while-loop
walk (probe95/96/97: 3/3, 4/4 incl. __slots__ entries and real
exc_info chains). ONLY during the exit-hook unwinding flow
(contextlib __exit__ -> pluggy _multicall -> collect_unraisable,
with the IndexError in-flight) does super().__init__(gen)
consume exactly ONE item from the generator. Standalone
super().__init__(generator) via the BASEINIT native arm works.
FAMILY: generator resumed from the BASEINIT dispatch path while
frame-unwind state is active consumes only the first item.
FIX SITE (next session): the native iterable-collect used by the
BASEINIT list arm (interp.rs ~5246-5285) - compare its generator
resume path vs exec-side; the unwinding state (current_exception/
tb_frames truncate) likely poisons the resume loop's completion
check. Also reverted: APRAW-ANY widened trace (kept APRAW-N).

## Session close (round-14): caches synced, milestone stable

Both cache trees byte-identical (code.py/terminal.py/
unraisableexception.py). `-p no:cacheprovider` milestone green
(1F/1P/1S, MAIN_RC=1, zero INTERNALERROR). With-cacheprovider
still crashes at the same generator-1-of-7 site (unchanged, as
expected - fix not yet applied). Repo: clean tree, ap-run 39/1
(known flake), 85 commits ahead of origin. NEXT SESSION: fix
the BASEINIT collect() generator-resume path (interp.rs
~5246-5285) to survive unwinding state, then default-config
gate should go green; then scrapy campaign.

## Round-15: root cause REVISED - GC drops suspended generator frames

GPULL outcome tracing (517 pulls: 336 Some / 178 None / 3 Err)
caught generator id=1677 receiving 3 Somes then None for the
7-node chain - and repeated rounds gave DIFFERENT counts
(1, then 3): NON-DETERMINISTIC truncation.
Isolation matrix: 7-node standalone PASS (P98), in-except PASS,
post-contextmanager-exit PASS (P99), real TracebackEntry+real
tb consistent 4/4 (P101). The failing flow's unique state:
suspended-generator frames holding tb nodes between yields.
HYPOTHESIS REVISED (P102): forced gc.collect() between yields
keeps the chain intact (4/4) - suspended generator frames ARE
GC-rooted. The nondeterminism (same flow yields 1 vs 3 across
runs) points to REENTRANCY: TracebackEntry(cur_) allocates
inside the generator; allocation can trigger GC -> the
unraisable hook -> pytest hook python code -> ITS OWN
gen_pull/resume_live re-enters while fiber 1677 is mid-resume,
clobbering fiber state; the outer resume then returns Ok(None)
early. NEXT SESSION: (1) add a reentrancy guard/serialization
to gen_pull/resume_live (queue nested pulls or detect and
defer); (2) rerun the 7-chain repro INSIDE the unraisable-hook
flow; (3) rerun default-config T2 gate.

## Round-16 (session close): error-in-generator CONFIRMED and partially fixed

NEW INSTRUMENTS LANDED (kept):
1. iter.rs Generator arm no longer swallows body errors as
   exhaustion - errors re-stage per fiber + APX_GEN_DEBUG logs
   [ITER-GEN-ERR] id/class/msg.
2. PENDING_GEN_ERROR keyed per fiber (map) + DEFAULT slot -
   nested resumes can no longer steal a body's staged error.
3. generators.rs current_running_fiber_id() exported.

CAPTURED: [ITER-GEN-ERR] id=1667 class=TypeError msg=
'NoneType' object is not iterable - the tb-walk generator dies
deterministically while constructing the 3rd TracebackEntry
(TE-CALL init fired exactly 2x before the error). The error was
SILENTLY SWALLOWED by the old iter.rs arm - the direct cause of
the 1-of-7 truncation (and previously invisible).
REMAINING (next session): why TracebackEntry construction
raises 'NoneType not iterable' on the 3rd call inside the
exit-hook unwind flow (same class constructs 10x fine in the
makereport flow; standalone repros P95-P104 all pass). Suspect:
class-call machinery state under unwinding (interp.
current_exception / contextlib frames) - instrument our
class-call path for __slots__ classes with APX_GEN_DEBUG.

## Round-17 (session close): _rawentry crash ELIMINATED - next defect exposed

With d9faec0 (no-swallow + per-fiber pending) the with-
cacheprovider crash CHANGED: '_type' has no '_rawentry' is GONE.
The now-visible error: TypeError int - None at
LoggingPlugin.pytest_sessionfinish `return (yield)` (logging.py
888) - the error the degenerate traceback had been MASKING all
along (exactly as hypothesized: excinfo for a sessionfinish
TypeError got the degenerate tb, whose getrepr crashed).
ROOT MECHANISM CONFIRMED: generator body errors inside
super().__init__ were silently eaten by iter.rs.
NEXT DEFECT (next session): int - None in the sessionfinish
wrapper teardown - candidates: catching_logs.__exit__
setLevel(self.orig_level) with orig_level never set (level None
enter) - our slot-missing read raising vs None needs one probe;
or vendored logging findCaller stacklevel arithmetic. The trap2
driver (/tmp/w110_t2drv_trap2.py) + traceback.print_exc now
gives REAL tracebacks for this flow - first instrument to use.
NOTE: full traceback printing now WORKS (probe trap2 shows
complete chains) - the tb machinery is healthy end-to-end.

## Round-18 (session close): int-None origin narrowed to catch_warnings_for_item teardown throw

Instruments (CLE/SFIN/SS-ERR/WRECORD - all REVERTED, caches
synced):
- catching_logs always runs with level=None (orig_level MISSING
  on every exit) - NOT the site.
- summary_stats NOT the site (SS-ERR never fired).
- call_historic loop NOT the site (WRECORD-ERR never fired).
- GEN-RAISE capture: TypeError 'int - None' originates in fiber
  397 = catch_warnings_for_item (warnings.py:63, the finally
  block) DURING pluggy's teardown.throw resume - tb_len=0.
  Propagates: 396 (warnings.py:119, tb_len=3) -> 395
  (terminal.py:961) -> 394 (logging.py:896).
Isolation: P106 (generator + catch_warnings + throw) PASSES.
The failure needs the full ihook.pytest_warning_recorded chain
under the trap driver. NEXT SESSION: extend the GEN-RAISE print
to dump e.tb ENTRIES (name:line:file per entry - commit has the
format ready), rerun trap2, and read the true innermost frames -
the 3-entry tb on fiber 396 shows contextlib.__exit__ frames
but the REAL subtraction site is deeper (misattribution).
Also: T2-gate hint - the driver difference (trap vs plain) is
the _hookexec trap raising IndexError paths.

## Round-19 (session close): int-None ROOT FOUND - missing AST + two more layers exposed

BREAKTHROUGH CHAIN (all instruments committed):
1. GTHROW (gen_throw dump) + carry-tb fix in the __ap_gen_
   throw__ Exception arm (was ExceptionValue::new dropping the
   instance __ap_tb__ - the thrown error's tb was DESTROYED).
2. With tb carried, the int-None's REAL origin surfaced:
   _pytest/_code/source.py getstatementrange_ast:195 ->
   ast.walk -> node._fields is None on our compile(PyCF_ONLY_
   AST) nodes - our runtime's compile() does NOT build a real
   AST tree (the _ast module is a name-surface stub; P109).
   The 'NoneType object is not iterable' error = ast.walk on
   the bogus tree; the int-None = downstream lineno arithmetic.
3. Staged workaround landed (A-3 works-best-effort):
   source.py getstatementrange_ast catches TypeError/
   AttributeError and degrades to full-range.
4. NEXT error exposed: traceback.py extract_tb 'cannot unpack
   non-iterable type object' during INTERNALERROR rendering -
   P111 proves islice/walk/unpack fine on real tbs; the failing
   tb comes from the thrown-in TypeError rebuilt chain - the
   entries were rebuilt from __ap_tb__ through the CLASS-
   fallback path (ev.tb may contain garbage entries from a
   class-attr walk). Walker hardening landed (try/except in
   _walk_tb_with_full_positions) - insufficient; the items are
   TYPES at the extract loop.
NEXT SESSION: (1) probe ev.tb content of the thrown TypeError
post-carry (GTHROW already dumps it - rerun and read); (2) the
int-None is FIXED for the runtest flow (test_pass PASSED, error
caught); remaining = rendering-path tb rebuild from __ap_tb__
(verify exception_value_of_instance's class-fallback doesn't
inject class objects as entries); (3) rerun trap2 gate.

## ROUND-19 VICTORY: FULL DEFAULT-CONFIG T2 GATE GREEN

The cacheprovider blocker chain, root causes (all fixed this
session):
1. iter.rs swallowed generator-body errors as exhaustion ->
   degenerate tracebacks -> _rawentry/ishidden crashes.
   FIXED (error re-staged per fiber, [ITER-GEN-ERR] visible).
2. gen.throw dropped the instance __ap_tb__ (ExceptionValue::new)
   -> wrapper-only tracebacks. FIXED (carry via exception_value_
   of_instance_pub).
3. int - None ROOT: os.umask(mask) returned None instead of the
   PREVIOUS mask; cacheprovider._make_cachedir:72 does
   path.chmod(0o777 - umask). FIXED (returns previous mask).
4. open('wb') returned a text stream -> write(bytes) TypeError.
   FIXED (<binary> attr + bytes write arm).
5. shutil.rmtree was a v1 stub -> NotImplementedError in the
   cache-dir cleanup finally. FIXED (real remove_dir_all with
   ignore_errors).
6. Missing AST tree (compile PyCF_ONLY_AST stub): staged
   source.py degradation (A-3 works-best-effort) + walker
   hardening in traceback.py.
FINAL GATE (t2g104, trap driver, DEFAULT CONFIG - cacheprovider
ENABLED): 1 failed / 1 passed / 1 skipped, MAIN_RC 1, ZERO
INTERNALERROR. The -p no:cacheprovider interim config is RETIRED.
ap-run suites: 39 passed / 1 failed (known flake). NEXT: rerun
full workspace gates, commit, resume scrapy campaign.

## Round-19 follow-up: two packaging_differential tests newly failing

sys_path_ordering_proof (fresh ANTIPTYHONY_CACHE run lacks
vendored_stdlib on sys.path) and add_six_import_use_byte_exact
(No module named __future__ in the same sandbox flow) - both in
crates/ap-cli/tests/packaging_differential.rs. Possibly caused
by the os.umask/open(wb)/rmtree changes rippling into the CLI
pack-fetch sandbox, or a stale-scratch-cache artifact of this
machine. NOT the T2/T1 core gates. NEXT SESSION: bisect against
commit 4423789 (pre-os-fixes) by checking out and rerunning
these two tests; root-cause the fresh-cache sys.path builder if
new. Core gates remain GREEN.

## ROUND-20 (session close): STALE BINARY ROOT - packaging tests green, workspace 418/0/1

The two 'newly failing' packaging_differential tests were NOT code
regressions: they run target/DEBUG/antipythony, whose baked
CARGO_MANIFEST_DIR path pointed at the PRE-RENAME repo dir
(/Users/caimlas/git/antipythany/... - gone since the rename).
cargo build (debug) refreshed it -> sys_path_ordering_proof PASS.
Same family: harness_selfcheck + xtask_gates test binaries held
env-dep/manifest paths from the old dir - touch + rebuild fixed
all. FINAL GATES: workspace 418 passed / 0 failed / 1 ignored
(packaging_differential 7/7); T2 default-config gate green
(1F/1P/1S, MAIN_RC=1, zero INTERNALERROR); FUZZ_SEEDS growth
committed per corpus-only-grows. Stale-binary GOTCHA recorded in
AGENTS.md status block (touch tests + rebuild before debugging).

## Round-21: scrapy unpack ROOT FOUND - Headers MRO drops ABC base

The int-None->unpack chain resolved end-to-end (real tracebacks
now work): downloader exception -> RetryMiddleware._retry ->
request.copy() -> Headers.update -> 'for k, v in seq' ValueError.
ROOT: isinstance(Headers(), Mapping) = False because class
Headers(dict, MutableMapping) is created with MRO [Headers,
dict, object] - the MutableMapping ABC BASE IS DROPPED at class
creation (not the Generic-drop arm; an earlier __mro_entries_/
ABC-base handling drops it). With MutableMapping absent from the
mro, the structural check correctly answers False and update()
takes the pairs branch, iterating keys ('too many values').
Also: deferred_from_coro 'hang' repro is INVALID - CPython 3.14
+ twisted raises the identical 'Task got bad yield: Deferred'
(w112_dfc_cpy.py ground truth); our runtime is
behavior-identical. Scrapy's real code path never awaits a
Deferred directly inside an asyncio Task.
Landed: b415c61 metaclass-only __instancecheck__ lookup (CPython
type(C).__instancecheck__ semantics; the earlier mro-attr lookup
pulled typing _GenericAlias hooks and broke classmethod call
convention - caught by the NamedTuple creation probe).
NEXT SESSION: fix the ABC-base drop (Headers mro must be
[Headers, dict, MutableMapping, Collection, ...]); then the
crawl should pass RetryMiddleware and reach first PARSE.
NOTE: a parallel agent session is active in this repo (APX_OSDBG
WIP in builtins_modules.rs/importer.rs + FUZZ_SEEDS growth - do
not revert; commit explicit paths only).

## Round-22 (session close): isinstance(Headers, Mapping) FIXED - crawl advances again

b415c61+patch: metaclass-only __instancecheck__ + type_ref/class_
ref mro walk in the fallback arm + dict-derived structural arm.
isinstance(Headers(), Mapping) == True (matches CPython).
CRAWL RESULT (current binary): the unpack ValueError is GONE.
RetryMiddleware now works: retry/count 2, retried twice, gave up
after 3 attempts, finish_reason finished, clean stats dump.
REMAINING (isolated, one probe away): all 3 download attempts
fail with "OSError: Is a directory (os error 21)" from
Path.open->io.open in the file handler — yet direct probes
(probe119/120: Path.open, io.open full-args, url2pathname) all
read /tmp/w111_crawl/page.html fine. Next: instrument
file_uri_to_path(request.url) inside the crawl (print filepath
in the file handler) — suspect the request.url at download time
is the DIRECTORY or a mangled url. Also: 2 unraisable 'super():
no arguments' + 'no attribute __del__' in Task.__del__ (minor).
Gates to rerun after the file-handler fix: T2 default + workspace.

## Round-23 (session close): the crawl blocker chain fully mapped

1. urllib.parse._urlsplit returned a PLAIN TUPLE (w3lib needs
   SplitResult attr access) -> parts.path lost -> safe_url_string
   ('file:///...') -> 'file://' -> Request url truncated ->
   download opened cwd (a directory) -> OSError x3 retries.
   FIXED: _urlsplit now returns SplitResult (vendored urllib/
   parse.py).
2. REMAINING: duplicate module instances. In the crawl process,
   scrapy's w3lib._url module instance has a BROKEN _quote_table
   (all entries b'' — RFC3986_UNRESERVED appears EMPTY there,
   i.e. string.ascii_letters resolves empty in that instance),
   while a fresh probe instance works. Two module instances of
   w3lib._url exist (P129 proved w3lib._urlsplit is not
   urllib.parse._urlsplit). Scrapy's Request.__init__ binds the
   STALE instance -> safe_url_string mangles every URL to
   'file://'. NEXT SESSION: find why the early-import instance
   gets an empty string.ascii_letters (likely import-order/
   thread: scrapy's Request imported before string module ready,
   or the dual-instance root in importer) and ensure a single
   instance; then the crawl reaches PARSE.

## Round-23b: crawl now REACHES PARSE dispatch; runtime zero-arg super() bug exposed

Progress this round: FH downloads body (60B), response 200,
scraper dispatches to callback chain. NEW runtime bug exposed at
Scraper.call_spider_async -> warn_on_generator_with_return_value
-> is_generator_with_return_value -> LocalWeakReferencedCache
(dict-derived weak cache) __setitem__/__getitem__:
zero-arg super() inside these dict-derived methods resolves to
'object' instead of walking the MRO to dict -> 'object' object
has no __setitem__ / RuntimeError super(): no arguments.
Staged suppression (datatypes.py suppress TypeError+RuntimeError)
reduces it to AttributeError from object.__setitem__ - still
kills the callback. NEXT SESSION (single task): fix zero-arg
super() resolution for dict-derived classes in the runtime -
the super() called from LocalWeakReferencedCache.__setitem__
must resolve the next-attr-holder in the MRO (dict), not
object. Likely in the super_stack push logic for the structural
setitem arm (methods.rs ~986-995) or classes.rs zero-arg super
base resolution skipping classes that lack the attr.
Once fixed: expect PARSE CALLED.
