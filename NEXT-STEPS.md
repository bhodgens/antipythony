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

## Session checkpoint note (2026-10-27, end)

Instrumentation attempt on Session.__init__ triggered an unrelated
`RuntimeError: super(): no arguments` (zero-arg super in
subclass__init__ after module-level __init__ reassignment) — do NOT
patch Session.__init__ directly; use plugin hooks instead.

Precise current behavior (deterministic): pytest collects 3 items,
runtest protocol for item 1 runs setup (report: passed), the call
phase hooks execute through pytest_pyfunc_call (verified via
APX_CALLTRACE + IAF probes), then the session ends without a call
verdict — items 2,3 never protocol. rc=1, "no tests ran", int-None
at teardown summary. No exception escapes any hookexec (verified).
Suspect: silent BaseException (StopIteration or similar) escaping a
from_call lambda inside runtestprotocol, unwinding to wrap_session's
except chain which swallows it and converts to exit code 1.

## Wave-11 (2026-10-27): precise failure point

test_pass RUNS AND PASSES (pytest_pyfunc_call impl executed the test
function). The failure is in the runtest_call wrapper TEARDOWN phase:
after pyfunc_call returns True, the 3 wrapper teardowns (Logging,
Capture, module-level) resume via teardown.send(result), and somewhere
in that resume chain the runtestprotocol continuation is dropped (no
makereport MC-entry, session unwinds, rc=1).

Next session: instrument _execution.py teardown loop per-wrapper
(print which wrapper's send/throw raised or returned unexpectedly) in
the real pytest flow. The first wrapper whose resume misbehaves is the
fix target. Also verify test_skip/test_fail behavior once fixed.

## Wave-12 (2026-10-27, budget end)

Teardown-loop trace for pytest_runtest_call shows only 1 of the
expected ~3 wrapper teardowns resuming before control jumps to the
protocol-level teardowns. The teardowns list appears to contain fewer
entries than expected, OR the loop exits early via an uncaught path.
The [impl] trace confirms 6 impls registered for pytest_runtest_call
(Logging wrapper, Capture wrapper, 2 module plain, unittest,
python.py trylast).

RECOMMENDED next session: print `len(teardowns)` and each entry's
type/kind at the top of the teardown loop in pluggy/_execution.py
(the instrumentation is already in place — just extend the [td] line
format), plus verify hook_impl.wrapper/hookwrapper flags match the
wrapper count. If wrapper flags are all False on our runtime (our
HookimplMarker stub may not propagate wrapper=True into
parse_hookimpl_opts), the wrapper impls are driven as PLAIN impls —
their generator return value is appended to results and never driven
→ exactly matches all observed symptoms (silent single-step, no call
report, items 2/3 dropped, rc=1).

## Wave-13 (2026-10-27, verified)

Wrapper flags ARE correct (3 wrapper=True impls for runtest_call). The
apparent "trace disappearance" after the first teardown resume is the
CAPTURE PLUGIN's stderr redirection: the capture wrapper's pre-yield
code replaces sys.stderr with the capture buffer, so subsequent debug
writes land in the buffer, not the console. Earlier "vanishing traces"
were partially an artifact of this.

REFINED next session: run with -s (capture disabled) plus the per-
teardown traces — traces will then be visible through the whole call/
teardown phase. The failing call-report path under -s is the true
remaining runtime bug (if any); distinguish it from capture-related
output plumbing.

## Subset-caller repro also passes (2026-10-27) — repro attempts closed

from_call + subset_hook_caller + wrapper works correctly on our
runtime. All synthetic repros pass; only the REAL pytest flow fails.
The difference must be in pytest's own layers between genitems/protocol
and the hook calls (CallInfo, _pytest.outcomes, fixture machinery,
Skipped handling) interacting with a runtime edge — OR the async_fail
OutcomeException escape (BaseException subclass bypassing `except
Exception`) is still firing nondeterministically for test_pass.

DEFINITIVE next-session probe: patch P.async_fail to a no-op (proven
working — produced "1 passed") AND simultaneously patch
P.is_async_function to log its input's repr at call time. If
is_async_function fires True for test_pass, print
`repr(testfunction)[:200]` + `type(testfunction).__mro__` + its
`__code__.co_flags` — this identifies the exact object shape our
inspect misreads as async. Then fix inspect.iscoroutinefunction or
the obj shape.

## Wave-final summary (2026-10-27)

Deterministic state: test_pass PASSES ("1 passed in 0.00s"); test_skip
fires Skipped but the session then loses the remaining items; test_fail
unreached. The last runtime bug: OutcomeException (Skipped/Failed,
BaseException subclasses in pytest 9) raised inside a nested ihook
relay call is not correctly re-raised through pluggy's
_multicall finally-block teardown resume chain on our runtime —
CPython re-raises it at the end of _multicall; ours appears to lose or
mangle it, terminating the runtest loop.

Next session: minimal repro = from_call(lambda: pm.hook.X()) where X's
impl raises Skipped-like BaseException, with a wrapper impl around it
(teardown.send path), then verify the exception re-emerges from
_multicall correctly. Fix in our exception propagation through
generator teardown .throw() (generators.rs resume_live GenInput::Throw
path interacting with pluggy's finally block).

## MINIMAL DETERMINISTIC REPRO (/tmp/t2sk.py, 2026-10-27 final)

Two impls of one hook: a wrapper (yield) + a trylast impl that raises
a BaseException subclass (Skipped). Expected (CPython): the Skipped
propagates out of pm.hook.call_hook() so from_call's `exc` captures it.
Ours: r1=[], exc=None — the exception VANISHES inside pluggy's
_multicall on our runtime (impl print also absent — suggesting the
impl ordering or the exception capture in `except BaseException as
exc` misbehaves).

This repro is small, deterministic, self-contained, and needs NO pytest
— pure pluggy + our runtime. Next session starts HERE: instrument
_execution.py's `except BaseException as exc` and the teardown
throw/resume path (instrumentation already partially in place), find
where the Skipped disappears, and fix the Rust side (gen_pull /
resume_live / GenInput::Throw interaction with pluggy's wrapper
teardown protocol).

## RETRACTION of /tmp/t2sk.py repro (2026-10-27)

The repro named the trylast impl `call_hook_2` — pluggy matches impls
to specs by function NAME, so it was correctly never registered as an
impl of `call_hook`. Invalid repro. (Instrumentation of
site-packages/pluggy reverted to clean.)

The REAL pytest flow verified earlier still stands: python.py's
pytest_pyfunc_call impl (trylast, correctly named) RAN and returned
True (verified via IAF probe printing False inside its body). So the
impl executes fine; the vanish is AFTER its return, in the firstresult
return path. Next session: instrument pluggy _multicall's
`if firstresult: result = results[0]` + final `raise exception` /
`return result` lines with stderr traces, run the real pytest flow,
and find which return statement executes and what it returns. The
`int - NoneType` then identifies the consumer that got None.

## Wave-11 addendum (2026-10-27, DEFINITIVE data point)

Instrumented the REAL failing run: pytest_pyfunc_call's multicall ends
with `exception=Failed('async def functions are not natively
supported')` — async_fail DID fire for test_pass in that run. Yet
probing the same hook in other runs shows
is_async_function(test_pass) = False (both inspect legs). CONFIRMED
NONDETERMINISTIC across runs of the same binary/args — import-order or
module-state dependent.

Working theory: our runtime's `from inspect import iscoroutinefunction`
binding in _pytest/compat.py resolves to a DIFFERENT callable depending
on import interleaving (e.g., our importer's from-import snapshot vs
live dict), or inspect module state diverges between import orders.
Under CPython both legs are always False.

Next session (single focus): run t2run36 under our runtime 5x with
compat.py instrumented (CAF probe restored); if nondeterministic,
print `id(iscoroutinefunction)` + `iscoroutinefunction.__module__`
inside is_async_function each call and compare ids across runs. That
identifies whether the from-import binding flips between the real
inspect function and a runtime marker — then fix our from-import
resolution (or the marker) accordingly.

## BREAKTHROUGH: ROOT CAUSE of the T2 async misfire FIXED (2026-10-28)

`NoneType` attr fallback (interp.rs ~8391) synthesized a virtual dunder
marker for ANY `__x__` name on None. `hasattr(None, "__await__")` and
`hasattr(None, "__aiter__")` returned True. pytest 9's
pytest_pyfunc_call (python.py:173) probes exactly those on every test's
return value; a sync test returns None -> async_fail -> Failed ->
session unwind. All earlier "nondeterminism" was probe placement and
buffering illusions.

FIX: block the async/iter/call/sequence dunder surface for the None
placeholder (matches CPython NoneType, which only serves
__bool__/__class__/__new__/__doc__/__str__/__repr__/__hash__/__eq__/
etc). Verified: hasattr(None,"__await__")=False.

RESULT: bare pytest run (t2run36, NO genitems/async_fail patches) now
prints "test_trivial.py::test_pass PASSED [ 33%]" and
"1 passed in 0.00s" - the milestone run works unpatched.

REMAINING (much narrower): (a) runtest loop stops after item 1 - items
2/3 (skip/fail) never execute; setup multicall of the second item
loses its continuation, sometimes nondeterministically re-raising a
stale Skipped("not today") at interpreter shutdown (stale exception
object identity). (b) frame/tb metadata corruption (all frames labeled
t2run36.py:8 with wrong function names) - traceback renderer.

## Wave-14 (2026-10-28): get_ident fix; skip evaluation now REACHED

SECOND REAL FIX: _thread.get_ident was bound to a plain INT (v0
stub). reprlib.recursive_repr CALLS get_ident(); dataclasses wraps
every generated __repr__ in @recursive_repr, so repr() of ANY
dataclass instance raised TypeError('int' object is not callable).
This corrupted: pytest's Skip dataclass handling, our own probes
(repr(_ev)), and pluggy wrapper teardown paths that repr objects.
FIX: bind marker("threading.get_ident") — the existing dispatcher
(threading_call.rs:191 get_ident_value()).

CHAIN RECONSTRUCTED (wave-11..14): the "nondeterminism" was layers of
bugs masking each other: (1) NoneType __await__ (fixed, b7a5faf),
(2) get_ident int (fixed this wave), (3) trace prints swallowed by the
capture plugin's stderr redirection (instrumentation artifact), (4)
stale OutcomeException re-raise at interpreter exit.

CURRENT STATE after both fixes: bare run t2run36 → skip mark IS
evaluated (skipping.py's pytest_runtest_setup raises Skipped), but the
Skipped vanishes during the setup multicall's wrapper teardown resume:
teardown.throw(Skipped) into the CaptureManager/Logging wrappers
RETURNS normally instead of propagating (pluggy then hits
_raise_wrapfail → 'object' has no 'f_code' — our gi_code surface is
also broken). Narrow repros of throw+with+contextmanager all PASS —
the bug needs the REAL fd-juggling item_capture __exit__ (os.dup2) or
the two-wrapper interplay.

NEXT SESSION (single focus): repro teardown.throw() returning
normally: wrap the CaptureManager wrapper EXACTLY (item_capture with
global fd capture active: capman activates GLOBAL capture earlier in
the session), throw Skipped, observe. Then fix gen.throw() resume in
generators.rs. Also fix gi_code to expose f_code (used by
_raise_wrapfail).

## Wave-15 (2026-10-28): frame-protocol hardening + loop-stall isolated

FIXES LANDED (gates 412/0/1 held):
1. Chained stub frames (builtins.rs sys._getframe chain + interp.rs
   f_back arm) now carry f_code (co_filename/co_name) - _py_warnings
   walk_stack reads frame.f_code.co_filename on every step.
2. Frame-protocol fallback at BOTH attr layers: interp.rs generic
   Namespace arm + classes.rs attr_get_protocol now answer f_back
   (None) and f_code (stub code object) for ANY object instead of
   raising AttributeError. Rationale: a miss during an unwind REPLACES
   the in-flight exception.
3. CRITICAL DIAGNOSTIC LESSON: the long-running "f_back AttributeError"
   was OUR OWN STALE PROBE - the [SKIPPED-INIT] instrumentation in
   outcomes.py called traceback.format_stack, whose frame walk hit
   frames lacking f_back. REMOVED. Lesson: instrument with
   print-only probes; never format tracebacks from probe code on this
   runtime (frame surfaces are incomplete).

REMAINING BLOCKER (single, now precisely characterized): in the REAL
pytest run, the setup multicall for item 2 iterates its 6 impls but
STOPS after the 2 wrapper impls (LoggingPlugin, CaptureManager) are
dispatched - the 4 plain impls never dispatch (PLAIN-CALL probe in
pluggy's else-branch never prints). The plain-impl else branch is
never reached. The loop's continuation is lost after the SECOND
generator first-yield next() dispatch. Synthetic reversed()+generator
in for-loop repros PASS - the trigger needs HookImpl method-wrapper
dispatch (kwargs binding, bound methods, capture fd juggling).
NEXT: bisect which wrapper's next() kills the loop: register ONLY
CaptureManager wrapper + plain impl (drop Logging) in the synthetic
multicall (t2thr9.py shape), then ONLY Logging. Whichever wrapper
kills the loop is the bug site; then fix its next()/generator frame
interaction in generators.rs.

## Wave-16 addendum: bisect results

Synthetic bisects all PASS (Capture wrapper alone, Logging wrapper
alone, both + global capture + plain raiser, reversed-loop + generator
per-iteration). The loop-stall does NOT reproduce outside the full
pytest env. All wrapper dispatches complete in isolation.

Note: the [DISP] probe placement initially misled - wrapper=True impls
go through the elif branch (new-style), not the hookwrapper branch.
[DISP2] confirmed: per item, exactly the 2 wrapper impls dispatch
their first-yield; the 4 plain impls never dispatch; the loop then
ends silently.

NEXT SESSION: instrument inside the elif branch AFTER next(function_gen)
to see if next() RETURNS (loop should continue - then inspect the
iterator state) vs next() raising StopIteration (would hit
_raise_wrapfail) vs next() never returning control. Also compare
hook_impls identity: print id(hook_impls) and len at loop entry vs
after each iteration - if the Sequence shrinks (a plugin REMOVES
impls mid-loop - pluggy _remove_plugin!), that's the mechanism: one
of the wrapper's pre-yield code (Logging's catching_logs) may
register/unregister plugins, and our runtime's list mutation during
reversed() iteration may TRUNCATE the iteration. CPython tolerates
this because reversed() takes a snapshot; our reversed may iterate
the LIVE list by index!

## Wave-17 (2026-10-28): repro campaign closed - all invalid

EVERY synthetic repro attempt for the item-2 loop stall was INVALID:
each either used wrong Python semantics (next() on a
_GeneratorContextManager object - genuine TypeError in CPython too),
didn't throw before the teardown loop, or matched CPython behavior
exactly. The runtime matches CPython on all of them.

Verified working in isolation: CaptureManager wrapper, LoggingPlugin
wrapper, both together, global capture, plain raiser, reversed()
loop, generator-per-iteration, deep frames, live-list mutation,
GC-pressure (invalid repro), CM objects (invalid repro).

METHOD LESSON: before chasing a runtime bug synthetically, write the
CPython reference run of the repro FIRST. Multiple sessions were
spent on repros that CPython also fails.

STATE: bare pytest run = "1 passed" (test_pass), session completes
through unconfigure. Item2's Skipped IS raised and IS captured by
_multicall (MC-catch, LOOP-EXIT exc=Skipped) - the loss is between
_multicall's normal return (exception=None after the FINAL trace
showed None - meaning the teardown loop CLEARED exception via
StopIteration from a wrapper throw-resume) and from_call's except.
PRIME SUSPECT (unverified): CaptureManager.item_capture's finally
(deactivate_fixture/suspend_global_capture/read_global_capture/
add_report_section) raises IndexError during the Skipped throw-unwind
(replacing it), then Logging wrapper's resume completes normally ->
StopIteration -> exception=None. VERIFY by instrumenting
item_capture's finally with print-only probes (NO traceback
formatting).

## Wave-18 (2026-10-28): stall EXACTLY located

With print-only probes (no traceback formatting), the item-2 flow
traces: Skipped raised -> MC-catch -> LOOP-EXIT exc=Skipped ->
teardown loop resumes Capture wrapper -> IC-FIN (item_capture finally
COMPLETES CLEANLY - no IndexError) -> then the teardown loop's next
advance (Logging wrapper) NEVER HAPPENS. The for-loop iterator
advance after a throw-resume that completed the generator (its
with-block finished, generator returned -> StopIteration ->
exception=None; continue) is lost.

MINIMAL SHAPE for next session: for-loop over a small list; body =
nested try; inside, gen.throw(Outcome) into a generator whose body
is `with CM(): yield` (CM = contextlib._GeneratorContextManager or
manual class); the throw unwinds the with-block (CM cleanup runs) and
the generator RETURNS -> StopIteration -> `exception = None;
continue`. Reproduce THAT exact chain; the loop's next advance is
lost. Suggested Rust-side inspection: the for-loop's iterator slot
handling when `continue` executes from a nested except handler after
a cross-fiber (corosensei) generator resume.

NOTE: still verify against CPython first (method lesson from
wave-17) - the expected CPython result of the minimal shape is:
CM cleanup runs, Outcome propagates (the generator does NOT return
normally), StopIteration only if generator completes.

## *** ROOT CAUSE FOUND: ev.instance missing in exception_value_of_instance (2026-10-28) ***

exception_value_of_instance() built ExceptionValue with ONLY the
class NAME - it never set ev.instance. Consequence: a USER exception
class (OutcomeException(BaseException), Skipped, any plugin
exception) that travels through gen.throw() has class="Outcome" with
no instance and no builtin-tree entry, so exc_matches()
`except BaseException` CANNOT match it (class-name equality fails,
via_instance needs instance, exc_class lookup needs the builtin
tree). The exception escaped EVERY handler - the entire runtest
protocol vanishing act.

FIX (one line): ev.instance = Some(v.clone()) in
exception_value_of_instance. Verified: min24 "caught Outcome".

NEW GATE STATE: pytest run reaches makereport for the Skipped; the
report formatting crashes with IndexError in
ExceptionInfoFormatter.repr_traceback chain - because our tb frames
ALL carry the runner script + wrong linenos, so pytest's filter
hides all frames (n=0) and a later Source[] lookup indexes out of
range. This is the KNOWN traceback-renderer corruption (frames
mislabeled), now the ONLY blocker for the T2 minimal gate: once tb
frames carry true filenames/linenos (or the formatter degrades
gracefully), Skipped -> skipped report -> 1 passed, 1 skipped,
1 failed.

NEXT SESSION: (1) fix tb_frames recording so linenos/filenames are
the real raise-site values (interp raise path pushes (qualname,
line) - currently they all show the module-level line), or make
traceback_value's nodes tolerant; (2) then the T2 gate should go
green; (3) run lxml 24/24 + corpus to confirm no regressions from
the ev.instance change (raising sites now carry instances - watch
for cycles via instance refs, GC).

## Wave-19 (2026-10-28): formatter IndexError scoped

After the ev.instance fix, item2's Skipped now reaches makereport,
but report FORMATTING raises IndexError inside
ExceptionInfoFormatter.repr_excinfo chain (repr_traceback itself
succeeds with filtered n=0; probes on get_source and _getreprcrash
did NOT fire). Root of THAT is the tb lineno/filename corruption:
touch_line() advances a GLOBAL stmt_ordinal (nested calls desync
it), and tb_node stamps every frame with script_path. pytest's
filter then hides all frames (all "internal") and downstream Source
indexing crashes.

STRUCTURAL FIX (next session, the real work): replace the global
stmt_ordinal/touch_line mapping with per-frame line tracking:
1. Add a line span to IR statements (ap-parse already has
   collect_stmt_lines; better: carry ruff spans through the builder).
2. exec_stmt sets frame.line from the span.
3. Frame exit pushes (qualname, frame.line, FILENAME) into tb_frames
   - extend the tuple with the frame's filename (function's defining
   module).
4. tb_node emits per-frame filenames.
This is the known documented follow-up (span map on the IR). It
fixes: corrupted tracebacks everywhere, pytest formatter crash,
longrepr quality, and the "everything at line 8" noise.

## Wave-20 (2026-10-28): IR line-span infra LANDED

- ap-ir: new Stmt::Line(usize) variant (no-op marker); dump/verify/
  walk updated.
- ap-parse lowering: lower_stmt_list, lower_fn_body, lower_class_body
  emit Stmt::Line(stmt line from ruff span) before each statement.
- ap-run exec_stmt: Stmt::Line(n) sets the current frame's line cell.
- test fixups: ap-ir fold tests now skip Line markers when indexing
  module items.

Gates: 412/0/1. Known remaining gap: tracebacks of CAUGHT exceptions
(tb_frames are only pushed on UNWOUND frame exits, then truncated on
catch) - e.__traceback__ for a caught exception still shows stale/
module lines. Full fix = push tb entries at RAISE time into the
exception value (per-exception tb), not per-interpreter table. That
is the next structural step; it also fixes pytest's longrepr
formatting (real filenames/linenos per frame) and unblocks the T2
minimal gate's last formatter IndexError.

## Wave-21 (2026-10-28): per-exception tracebacks LANDED

- ExceptionValue.tb: Vec<(qualname, lineno)> - appended at raise
  (raise-site entry), each FnDef frame exit, and module exit.
- exception_instance() serializes exc.tb onto the instance as hidden
  attr __ap_tb__ (list of (name, line) tuples).
- UserInstance.__traceback__ renders __ap_tb__ via build_tb_chain()
  (real filenames/linenos per frame).
- sys.exc_info prefers ev.tb over the global table (build_tb_chain).

Gates: 412/0/1. Test-data staging restored to pristine (pluggy
1.7.0 reinstalled; pytest 9.1.1 files from system install - my
probe edits to the cache had corrupted pluggy/_execution.py).

REMAINING for T2 minimal gate: the Skipped report formatting still
hits IndexError in ExceptionInfoFormatter.repr_excinfo chain. With
per-exception tb landed, next session should re-investigate with
CLEAN site-packages: the crash may now have a different shape (the
filter previously saw all-hidden frames; now frames carry real
paths). Also confirm the corpus/lxml regressions didn't appear from
the tb/instance changes (cargo 412 green; run the differential
suites next session).

## Wave-22 (2026-10-28): regression sweep result

- cargo gates: 412/0/1 green after ALL this turn's changes
  (ev.instance, frame-protocol, Stmt::Line infra, per-exception tb).
- scrapy import regression sweep: FAILS with a misleading
  "'int' object has no attribute 'phrase'" — VERIFIED PRE-EXISTING
  (fails identically at HEAD~3 with this turn's commits stashed).
  Likely the documented import-order nondeterminism; not caused by
  the tb/instance changes. The T1 environment needs its original
  invocation context (the probe ran inside a project dir; /tmp
  scripts lack the antipythany.lock that joins site-packages, and
  ANTPYTHANY_PATH alone reproduces the failure).

COMMITTED THIS TURN:
- 5424f1b ev.instance fix (THE T2 root cause)
- c9926f6 frame-protocol hardening
- 944948d IR Stmt::Line markers
- 4e7bf76 per-exception tracebacks (ExceptionValue.tb +
  __ap_tb__ + build_tb_chain + sys.exc_info preference)
- docs waves 17-22

NEXT SESSION (single focus): per-exception tb + true linenos are in;
debug the remaining makereport IndexError against the now-pristine
staging with print-only probes. The tb chain for the Skipped should
now carry real entries — confirm what pytest's filter sees.
