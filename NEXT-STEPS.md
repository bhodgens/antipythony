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
