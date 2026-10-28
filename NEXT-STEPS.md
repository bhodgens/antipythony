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
