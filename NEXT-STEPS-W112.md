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
