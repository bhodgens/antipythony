# T2 Next Steps — 2026-10-27

State: T2-b agent (deleg_8a29e236) died to a provider outage after 37h.
Its uncommitted work (210 lines: pluggy dir() surface, plugin __qualname__
rewrite, int(ExitCode)) was verified and absorbed into commit f2123fd.
Follow-up fixes in 63a80f7.

## Verified working now (commit 63a80f7)

- pytest.main: parses args, resolves rootdir, registers all 30 default
  plugins, reaches collection, reports the session header and exit code.
- pathlib: Path.glob / iterdir / os.scandir iteration all work.
- set/frozenset membership for user-__eq__ members (Path objects).
- SystemExit.code (int conversion for pytest exit codes).
- genexp/map/list lazy drains; scope_of memoized without the ABA bug.
- Gates: cargo test --workspace 412 passed / 0 failed / 1 ignored;
  lxml_differential 24/24.

## Fixed in 62b2885 (2026-10-27, wave 2)

- staticmethod type identity: `type(sm) is staticmethod` was False
  (builtin_type created a fresh Type per call) → vendored enum.py's
  double-wrap guard failed → staticmethod(staticmethod(f)) → 'object'
  object is not callable in _simple_enum. type(x) now returns the
  cached interned type; UserClass returns its own cached class Value.
- os.scandir(Path(...)) silently scanned "." (relative DirEntry paths
  broke Session/Dir collection matching). Now coerces PathLike args
  via __fspath__.
- os.umask: real syscall arm (pytest cleanup needs it).

Probes green: /tmp/t2ident.py (type identity), /tmp/t2nonlocal3.py
(_simple_enum builds Color), /tmp/t2prop.py (@property), t2sm2.

## Fixed in cd3602b (2026-10-27, wave 3)

- _colorize Theme traceback sub-namespace fields (traceback render now
  works - this was masking every error's location).
- isinstance(x, A | B) union support (the 0-items collection blocker:
  istestfunction's isinstance(obj, staticmethod | classmethod) raised).
- user-instance __delitem__ (del config.stash[key]).
- threading.ExceptHookArgs constructor.
- dict.popitem (pytest SetupState.teardown_exact).

**MILESTONE: pytest.main collects 3 items and EXECUTES tests**
(test_trivial.py::test_pass runs; RC=1 with remaining teardown issue).

## Current state after cd3602b (next session targets)

1. `int - NoneType` during runtest teardown/verdict recording: after
   test_pass executes, the protocol dies silently (rc=1, "no tests
   ran"). The error is caught by pytest's report machinery - trace via
   the HOOKFAIL wrapper pattern (/tmp/t2run17.py) on
   pytest_runtest_teardown / pytest_runtest_makereport hooks.
2. Session.genitems lazy-generator resume bug (issue-#7 class): the
   original generator breaks under items.extend(); eager list()
   workaround works. Root-cause in the generator fiber resume path.
3. Kw-only call syntax `f(*, a=b)` is a parser SyntaxError - parser gap.
4. inspect.getsource broken (tokenize) - separate issue.
5. flaky parallel test: circular_imports_partial_visibility - use
   `cargo test --workspace -j 4` (412/0/1 reliable).
6. /tmp cleaners: recreate lxml-ref-venv before lxml gates.

pytest.main completes parse, collection, session setup AND reaches
Config._ensure_unconfigure teardown. Remaining error in cleanup:
`TypeError: unsupported operand type(s) for -: 'int' and 'NoneType'`
raised from _pytest/threadexception.py cleanup →
collect_thread_exception → Stash[thread_exceptions].pop path
(deque.pop on our runtime raised correctly in isolation; the int-None
comes from somewhere inside the hook/stash interplay — needs
isolation). Also the traceback renderer still misattributes frames
('object' object has no attribute 'type' inside TracebackException) —
fix that FIRST, it masks every error's real location.

Note: cargo test full-workspace has a flaky parallel-suite failure
(circular_imports_partial_visibility passes in isolation; suspect
fixture/target-dir race). Use `cargo test --workspace -j 4` for
reliable gates (412/0/1).

## Queued (unchanged)

- issue #10: GC 3-case regression fix wave.
- threading suite: 1 timeout case.
- A-2 adaptations inventory items 4-10 (RustPython donor backlog).
- faulthandler.dump_traceback_later is a no-op (separate issue; write up).
- inspect.getsource broken (tokenize) — separate issue.
- /tmp/lxml-ref-venv gets wiped by /tmp cleaners and loses pyvenv.cfg;
  recreate with `uv venv /tmp/lxml-ref-venv --python 3.14` +
  `uv pip install -p /tmp/lxml-ref-venv lxml` before lxml gates.

## Wave-3 addendum (late finding, 2026-10-27)

pytest_pyfunc_call raises "async def functions are not natively
supported" for the SYNC test_pass. Traced:
- inspect.iscoroutinefunction(test_pass) = False inside the hook.
- The True must come from the `inspect.isasyncgenfunction(func)` leg of
  _pytest.compat.is_async_function, OR pyfuncitem.obj differs from the
  plain function under hook context.
- CRITICAL related finding: monkeypatching a module attribute after
  import does NOT affect already-defined functions in our runtime
  (globals resolve through FnRegistry captured at definition, not a
  live module-dict lookup). This breaks unittest.mock.patch, pytest
  monkeypatch.setattr on modules, and all orchestrator debugging via
  monkeypatching. CPython semantics require globals lookup to go
  through the module __dict__ at call time. This is a fundamental
  interpreter fix worth its own leaf.

## Wave-4 finding (2026-10-27, latest)

Runtest execution narrows to: pytest_runtest_protocol enters for item 1
(shouldstop=False), setup report logs passed, pytest_pyfunc_call body
runs (is_async_function verified False via instrumentation), and then
the call-phase completes SILENTLY — no call report, protocol never
returns, run ends 'no tests ran' rc=1.

Root cause hypothesis (high confidence): the runtest protocol runs the
call phase through pluggy hookwrappers (generators). Our runtime drops
or prematurely exhausts the resumption chain when the call function
itself executes through the interpreter — i.e. the nested
hookwrapper-generator resume bug (issue-#7 class). Evidence:
- from_call entered for 'call', never logged ok/error, yet pytest
  continued to session summary.
- Hand-written deep chains (lambda → method → assert) work fine.
- HookimplMarker in our pluggy stub lacks wrapper/hookwrapper kwargs —
  terminal.py's @hookimpl(wrapper=True) must be falling back to a
  plain-impl path, breaking the wrapper protocol silently.

Next concrete steps:
1. Extend HookimplMarker/make_descr to accept wrapper= and hookwrapper=
   kwargs and mark the descriptor kind (pluggy stub in builtins_modules).
2. In pluggy's _multicall (site-packages, test-data-only), verify
   wrapper impls are detected via impl.wrapper attr and driven as
   generators (send/throw protocol) — instrument if needed.
3. Re-run /tmp/t2run14.py: expect 1 passed, 1 failed, 1 skipped, RC=1.

## Wave-4 addendum 2 (2026-10-27)

Isolated the empty-collection mechanics: pluggy registration + call
SUCCEED structurally (pm.hook.outer_hook exists) but _multicall runs
ZERO impls (r=[]). Prime suspect: pluggy's varnames()/HookImpl
argnames extraction inspects function signatures via our code-object
surface; if argnames come back empty the impl is skipped as
non-matching. Verify: instrument pluggy/_execution.py _multicall with a
stderr print of hook_impls length (site-packages is test-data-only) —
if 0, the gap is in HookImpl._from_hookimpl/varnames; if N, the gap is
in the wrapper-drive loop.

## Wave-4 addendum 3 (CONFIRMED root, 2026-10-27)

pluggy add_hookspecs(module) RAISES 'did not find any hooks' — hookspec
discovery iterates the module's attributes and finds NONE. Our native
Module objects (ObjKind::Module) do not enumerate their namespace
through the paths pluggy uses (dir(module)/vars(module) → attribute
getattr chain). The earlier 'working' run used a CLASS as hookspec
holder (class attr enumeration works) which is why pytest config-time
hooks (registered via class instances) ran, but module-style plugin
registration yields zero hookimpls → every ihook call returns empty →
silent skips everywhere (collection reports, runtest protocol,
verdicts).

Concrete fix (next session): make dir(module) / vars(module) /
getattr-chain enumeration work for ObjKind::Module values, OR wire
pluggy's add_hookspecs plugin-name discovery through module_ns directly.
Test: /tmp/t2hw4.py (add_hookspecs must not raise; impl count == 2).

## Wave-4 addendum 4 (2026-10-27, correction)

The add_hookspecs 'did not find hooks' repro was INVALID: the probe
used a bare function without the @HookspecMarker decorator, so pluggy
correctly found no specs. dir()/vars() enumeration on modules and
__main__ works fine in our runtime. Retract that root-cause claim.

Validated facts that still stand:
- isinstance(x, A|B) works (wave-3 fix).
- pytest collects 3 items and reaches pytest_pyfunc_call for item 1;
  is_async_function(test_pass)=False verified inside the hook.
- The call phase still vanishes silently after pyfunc_call runs; the
  int-None and 'no tests ran' remain unexplained. The generator
  resume bug (issue-#7 class) remains the prime suspect: instrument
  pluggy/_callers.py _multicall old-style and new-style wrapper
  driving (teardown.send / next(teardown)) in the pytest process
  context next session.

## Wave-5 narrowing (2026-10-27, final this session)

Precise trigger isolated: patching ONLY Session.genitems to eager
list(orig(self)) makes collection produce all 3 items and run tests.
The ORIGINAL lazy generator drops items when DRIVEN through pytest's
nested machinery (ihook multicall + from_call frames), while:
- the same genitems logic in a hand-written repro works,
- union isinstance works inside generators,
- pluggy wrapper protocol (pre-yield/plain/post-yield) works,
- bare generator send() works.

The bug is the def-generator fiber resume when its caller frame chain
includes native dispatch layers (our call_value_pub -> ihook -> _multicall
-> wrapper gen). Root fix belongs in the generator/fiber resume path
(crates/ap-run generators.rs) — likely frame state loss across the
native-call boundary. Suggest: add a debug env var tracing
gen_pull/resume_live with generator ids + call depth to catch where the
lazy genitems' frames drop their children.

## Wave-6 ROOT CAUSE FOUND (2026-10-27, commit 943dca9)

PyIterator::next() returned None for IterState::Generator ("handled in
interp.rs, never via this method"). Every collector draining via
it.next() — snapshot()/collect() in builtins_modules (list.extend,
tuple, set, sorted, dict.update) — silently got ZERO values from any
def-generator. perform_collect's
`self.items.extend(self.genitems(node))` appended nothing; runtest loop
had no items; session ended 'no tests ran'.

FIX: next() routes Generator(id) pulls through
builtins_modules::current_interp + gen_pull_public (crate-internal,
legal since iter.rs lives in ap-run). Probes: extend/list/tuple/set/
sorted/dict.update all drain generators correctly.

## Next session target (precise)

pytest now collects 3 items. Remaining single issue: runtest protocol
for item 1 completes setup (report logs passed) and executes the call
(verified pytest_pyfunc_call entered, is_async False) but the session
ends silently afterwards ('no tests ran', rc=1, int-NoneType TypeError
at top level). Next: trace the item.ihook pytest_runtest_makereport /
pytest_runtest_logreport chain for item 1 — the call report is created
but not logged, or the logreport hook vanishes. Tools ready:
/tmp/t2run22.py (hook-fail tracer), /tmp/t2run28.py (protocol/report
tracer), /tmp/t2run27.py (top-level exception walker).

## Wave-6 addendum (2026-10-27, end of session)

Traced every pluggy hook call (HookCaller.__call__ instrumented).
Sequence for item 1: ... setup → makereport → logreport → teststatus →
pytest_runtest_call → pytest_pyfunc_call → **pytest_sessionfinish**.

The makereport-for-call and logreport-for-call hooks NEVER fire:
runtestprotocol's continuation after the call-phase from_call is
dropped. from_call(lambda: ihook.pytest_runtest_call(...)) — the lambda
runs the wrapper chain to completion (pyfunc_call verified), but the
return into runtestprotocol vanishes, so the call report is never made
and pytest ends the session.

Minimal repro attempts (single wrapper + follow-up hook) all PASS — the
vanish requires pytest's real structure: runtestprotocol → from_call →
lambda → ihook.runtest_call (wrapper) → impl → item.runtest() → nested
ihook.pytest_pyfunc_call → testfn, i.e. TWO nested ihook relays with a
wrapper at each level and a plain return through both.

Next session: build the two-level nested ihook repro (Function-style
per-node HookRelay), then fix the fiber/return propagation through it.
Also verify with the int-None walker (/tmp/t2run27.py pattern) once the
call report exists.

## Wave-7 (2026-10-27, session end)

Further narrowing:
- item.runtest() IS called ([runtest] test_pass traced via
  Function.runtest patch).
- Inside it, item.ihook.pytest_pyfunc_call(...) is the call that
  vanishes. Traced: HookCaller.__call__ for pytest_pyfunc_call FIRES
  (from HC-call log), the impl runs (is_async probe printed inside
  python.py's pytest_pyfunc_call body), then the RELAY RETURN path
  vanishes — runtest() never sees the result, runtestprotocol never
  proceeds, loop exits, rc=1.
- The return path crosses: pluggy _multicall firstresult return →
  HookCaller.__call__ return → Function.runtest return → pytest_runtest
  _call impl return → wrapper teardowns → runtestprotocol. One of these
  returns drops the whole continuation (not just the value).
- Repro attempts with two separate PluginManagers did NOT reproduce
  (works fine) — real pytest uses ONE 'pytest' project with
  Session-level and item-level HookRelays; repro must use a single pm
  with per-item hook relays emulating _pytest.hookspec + node
  construction.
- int-NoneType TypeError at session end: likely
  terminal summary duration math (self._session_start.elapsed()) when
  sessionstart partially completed, secondary to the vanish.

Next session: single-plugin repro with Session.collect/Item.runtest
split across two HookRelays on one 'pytest' project name; trace
gen_pull/resume_live around the pyfunc_call firstresult return.

## Wave-8 (2026-10-27, final)

Deterministic repro captured: with Function.runtest wrapped, runtest()
for test_pass ESCAPES with `Failed: async def functions are not
natively supported.` — raised by python.py's pytest_pyfunc_call
because is_async_function(pyfuncitem.obj) returned True for the sync
test_pass. In pytest 9, OutcomeException inherits BaseException, so it
bypasses runner.py's `except Exception` and unwinds the whole session —
hence rc=1 with no verdicts.

Direct probes of the same function object (import_path → mod.test_pass)
show iscoroutinefunction/isasyncgenfunction both False. The True must
arise from pyfuncitem.obj being a DIFFERENT object at hook time (fixture
wrap) or our iscoroutinefunction mis-reading the wrapped object.

Next session, precise plan:
1. In the pytest process, patch python.py's async_fail binding (module
   global at call time works — verified binding is live) to print
   `repr(pyfuncitem.obj)` and `type(obj).__mro__` BEFORE raising.
2. If obj is a fixture-wrapped partial/marker: fix our
   iscoroutinefunction/isasyncgenfunction (or functools partial marker
   handling) for that shape.
3. If obj is the plain function: our is_async_function's second leg
   (isasyncgenfunction) misfires under pytest's import context — diff
   inspect module state between standalone and pytest contexts.

## MILESTONE (2026-10-27, final): "1 passed in -0.00s"

With P.async_fail patched to return None instead of raising, the full
pytest.main run completes: **1 passed** (test_pass), test_skip's skip
fires ("Skipped: not today" in output), session completes. pytest runs
real tests on our runtime for the first time.

Remaining single issue: python.py's pytest_pyfunc_call calls
async_fail(nodeid) for the SYNC test_pass — is_async_function(obj)
returns True inside the hook while standalone probes return False.
Since P.async_fail's module-global patch works (binding is live), the
same technique on P.is_async_function should reveal what obj is at
raise time (repr + mro). The t2run40 attempt added the trace but a
later crash (Skipped path) obscured it — rerun with BOTH patches
(async_fail = no-op AND is_async_function traced) to get the answer,
then fix is_async_function or the obj shape.

The 'in -0.00s' negative duration is cosmetic (duration math with
None/0 — trace sessionstart Instant wiring if it bothers anyone).

## Determinism check (2026-10-27, session end)

- With eager genitems + async_fail no-op: "1 passed" — milestone
  reproducible.
- WITHOUT the eager patch (original lazy genitems): collection reports
  3 items but the runtest loop runs only item 1's setup, stalls at
  test_pass, 'no tests ran', rc=1 — deterministic across 3 runs.
- So the remaining runtime bug: the RECURSIVE yield-from genitems
  generator (Session.genitems → yield from self.genitems(subnode)),
  driven via PyList.extend through the new IterState::Generator pulls,
  completes collection but the recursion + nested ihook calls
  (pytest_itemcollected) inside it drop later items or corrupt the
  runtest loop state. Simple (non-recursive) generator drains work.

Next session: add a depth/trace env-var to gen_pull/resume_live, run
the runner3 flow with tracing, find the exact recursion depth where
the lazy genitems generator misbehaves vs the eager path. Likely the
fiber's parent-frame resume loses the outer genitems generator's
position when a nested ihook (pytest_itemcollected) re-enters the
interpreter.

## MINIMAL REPRO FOUND (/tmp/t2yf13.py, 2026-10-27)

Recursive yield-from generator + pm.hook.<name> calls inside, driven
by extend()-style iteration: hook impls registered on the same pm DO
NOT FIRE (fired=0) and the recursion collapses (items=1, expected 3).
The hook relay call inside the generator fiber silently no-ops.

Repro essentials: add_hookspecs(module); register(impl); generator
function that calls pm.hook.X() and recursively yields from itself;
drive with a for-loop. Compare CPython: fires=3, items=3.

Next session: instrument pm.hook.X attribute path (HookRelay.__getattr__
→ HookCaller) INSIDE the generator fiber — the relay lookup likely
returns a stale/empty HookCaller because the generator fiber's module
globals/context differ, or gen_pull's fiber resume loses the relay
binding. Fix, then pytest gate completes.

## Wave-9: repro CORRECTED and boundary pinned (2026-10-27)

The minimal repro (pm.register(bare_function)) was invalid — pluggy
correctly skips bare functions lacking the hook-name attribute. The
faithful repro of the pytest failure is:

- a MODULE (or class) with @hookimpl-marked methods registered via
  pm.register(module_or_class) — impls fire outside generators;
- Session.genitems-style generator that calls pm.hook.<spec>()
  inside recursive yield-from, driven by list/extend iteration —
  inside the generator fiber, the identical hook call resolves the
  HookCaller but iterates ZERO impls (or never reaches _multicall).

Boundary pinned: the vanishing happens only when the hook call is made
from inside a generator fiber resumed via the new IterState::Generator
pulls. Single-level and wrapper chains outside generators work.

Next session: dump HookRelay identity + HookCaller._hookimpls length +
_multicall entry from INSIDE the genitems fiber in the REAL pytest flow
(t2run41-style tracing but on pytest_itemcollected), then compare the
relay object identity across the fiber boundary — the bug should be a
stale relay/context in the fiber, or gen_pull's fiber executing with a
fresh/empty interpreter state (CURRENT_INTERP thread-local not
propagated into the resumed fiber → pluggy's hookexec closure or the
impl enumeration reads a fresh registry).

## Wave-9 addendum: thread-local check (negative, 2026-10-27)

Generator fibers run via corosensei stack-switching on the SAME OS
thread (generators.rs ACTIVE/PARKED thread-local stack), so the
CURRENT_INTERP thread-local is preserved across resume — the
hypothesis "fresh interpreter state in the fiber" is WRONG.

Refined next-step: inside the genitems fiber, log
`id(pm.hook.item_collected._hookimpls)` and the pluggy
PluginManager._hookimpls dict identity vs the same objects outside.
If they match, the dispatch reaches _multicall and the IMPL LIST is
the thing that's empty at that point → then instrument pluggy's
parse_hookimpl_opts: OUR runtime's getattr(plugin, name) inside
parse (plugin=runner.py module) may fail for module-level hookimpl
functions because OUR module attr lookup on ObjKind::Module during
FIBER execution skips BuiltinFn/Function-kind values created by the
importer (vs regular attribute reads outside fibers).

## Wave-9 final (2026-10-27): repro lessons

Two more invalid repros (impl name mismatch; registration timing) —
retracted. Building a faithful pluggy repro requires mirroring
pytest's structure exactly. RECOMMENDED APPROACH for next session:
skip synthetic repros. Instead, run the real pytest flow with
pluggy/_caller.py and pluggy/_execution.py instrumented (stderr prints
of hook name + len(hook_impls) + HookCaller identity) — the previous
instrumentation caught pytest hooks firing OUTSIDE genitems but we
never verified whether pytest_itemcollected's _multicall runs INSIDE
the genitems fiber with a non-empty impl list. That single data point
(decide: empty-impl-list vs never-called vs called-but-result-dropped)
directly selects which runtime layer to fix.

## Wave-9 final resolution (2026-10-27)

DECISIVE: explicit `list(session.perform_collect())` inside a
pytest_collection plugin yields items=3, session.items=3 — COLLECTION
IS FULLY WORKING through the eager perform_collect body (the itemcollected
impls=0 lines are normal: no plugin implements that hook in a bare
install).

The remaining failure is purely in the RUN phase: pytest_runtestloop →
pytest_runtest_protocol(item 1) → setup passes → call executes
(pytest_pyfunc_call verified entered and returning) → the call REPORT
chain vanishes → 'no tests ran', rc=1, int-NoneType surfaced.

Next session focus (single narrow target): instrument
runner.py call_and_report / pytest_runtest_makereport for the 'call'
phase (CallInfo.from_call enter/exit already proven to enter) — find
where between 'pyfunc_call returns True' and 'logreport(call)' the
continuation drops. Suspects: our runtime's StopIteration-as-value
handling in wrapper teardown resume (si.value may come back None),
or _Result.__force_result arithmetic.

## Wave-10 (2026-10-27, session end): vanish precisely characterized

Instrumented HookCaller.__call__ + _multicall for every call. Full
sequence: [setup makereport → setup logreport → teststatus] →
[runtest_call MC → 3 wrappers → pyfunc_call MC] → **sessionfinish**.
NO exception escapes any hookexec (HOOKESC tracer silent). The call
verdict hooks (makereport/logreport for 'call') never fire.

=> runner.py's runtestprotocol stops executing right after
`call_and_report(item, "call")` returns: the very next statement
(`rep = ihook.pytest_runtest_makereport(...)`) never runs, and control
appears to return past runtestprotocol directly to wrap_session
(sessionfinish fires; 'no tests ran'; rc=1). The 'int - NoneType'
TypeError is session-teardown duration math running with partially
initialized state — a SECONDARY symptom of the same lost continuation.

Working hypothesis: our runtime's function-return path unwinds to the
wrong frame when a python lambda (from_call's arg) calls through a
native dispatch layer (ihook → _multicall → wrapper gen resume) that
itself executes interpreter code — i.e., a frame-return addressing bug
across the native/python boundary in deep mixed stacks.

Next session: add APX env-gated tracing to call_value return paths
(print frame depth + function name on entry/exit), run t2runner3, and
compare the observed unwind against the expected one. The first
'wrong frame' jump localizes the bug in interp.rs call/return
handling.

## Session end state (2026-10-27)

Exhaustive call-level tracing (APX_CALLTRACE) shows ALL hooks returning
"cleanly" (generator values are correct — logging.py/terminal.py
wrappers, pluggy drives them properly). No exception escapes any
hookexec. The runtest loop runs item 1's protocol (setup report logged
passed) then exits the loop silently: items 2,3 never protocol'd, no
call report for item 1, session.items.count discrepancy, rc=1.

Prime remaining suspect (unexplored): session.shouldstop /
session.testsfailed state corrupted during item 1's protocol (int
arithmetic on a None), causing main.py:409-412 (`if session.shouldfail:
raise` / `if session.shouldstop: raise`) to fire — with the raise
caught by wrap_session and converted to a silent exit. Next session:
instrument Session.shouldstop/shouldfail/testsfailed writes (they're
plain attrs; patch Session class) and log every write. The first
unexpected write identifies the culprit.

Also unexplored: pytest_untrunnable / unraisableexception plugin's
sessionfinish arithmetic (int - None) may itself BE the loop-exit
trigger via an exception swallowed by wrap_session's except chain.
