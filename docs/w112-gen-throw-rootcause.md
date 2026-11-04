# W112 session notes — generator-throw-through-native-dispatch bug
# (pluggy _multicall shim). 2026-11-03.

## Verdict: NOT FIXED; root cause narrowed substantially

The bug is now reduced to a ~30-line deterministic, file-name-sensitive
repro and several wrong hypotheses were eliminated. The shim still
cannot be dropped.

## THE MINIMAL REPRO (new, verified)

/tmp/w112gen_y144.py (+ ordering probe /tmp/w112gen_y145.py) —
self-contained, no pytest:

1. Build a real pytest-like config via `_pytest.config._prepareconfig`,
   FixtureManager, session.perform_collect (needs session._fixturemanager
   set BEFORE the collection hook — see note below).
2. Trial A: `CallInfo.from_call(lambda: ihook.pytest_runtest_call(item=item),
   when="call", reraise=...)` executed inside a function compiled with
   `co_filename` UNDER ~/.cache/antipythony/site-packages
   (e.g. .../site-packages/_pytest/runner.py).
3. Trial B (immediately after): `ihook.pytest_runtest_makereport(...)`
   from an sp-named frame.

A passes; B raises IndexError('list index out of range') instead of
relaying the AssertionError. Deterministic. ORDER MATTERS: B-first with
from_call in a /tmp frame passes (y145); A-in-sp-frame followed by B
fails (y144). Same bodies compiled with /tmp filenames always pass.
Driver `w112_t2drv_noshim.py` (shim-free pytest.main) still dies with
the old IndexError at _multicall:194. The surfaced IndexError is NOT
the AssertionError reclassified (isinstance AssertionError == False);
it is a NEW instance built by the runtime.

## Filename rule (from y100/y102/y103 probes)

Compiled co_filename under .../site-packages/_pytest/<name>.py or
.../site-packages/pluggy/<name>.py → FAIL. Same-named file copied to
/tmp → PASS. .../site-packages/aaa.py (unknown pkg) → PASS.
.../site-packages/q/w/e.py → PASS. /opt/homebrew venv paths → PASS.
I.e. the trigger is a co_filename inside an EXISTING registered package
dir of the staged site-packages — strongly implicates importer.rs's
per-module state (IR line tables / fn registries keyed by path), not
the code itself.

## Wrong-turn log (do NOT re-chase)

- Pure pluggy synthetic repros (wrapper+raiser, firstresult,
  Outcome(BaseException), hookwrapper old-style, nested wrappers,
  with-blocks, from_call shape, repeat calls, def-nesting depth): ALL
  pass on our runtime. The bug needs real pytest plugin state.
- `_multicall` body is NOT at fault: byte-identical verbatim copy of
  pluggy 1.7.0 _multicall compiled in ANY other module namespace makes
  the real pytest gate pass (verb test). Only the importer-compiled
  original fails.
- `results[0] if results else None` (line 158) is NOT the failing
  expression (len-guarded variants also fail); the tb points at
  _multicall:194 `raise exception` — and our runtime RAISES IndexError
  where CPython raises the stored AssertionError, i.e. the exception
  instance/class is corrupted or replaced in the throw relay.
- NOT list-subscript: APX_IDX_DEBUG instruments methods.rs
  subscript_get list arm, list-delete arm, both raise() helpers, and
  the EXCS ctor — the failing flow fires NONE of them, yet surfaces
  IndexError('list index out of range'). So the IndexError instance is
  REBUILT somewhere outside these paths — most plausibly the thrown
  AssertionError instance loses its identity in the generator-throw
  relay (ev→instance→ev conversion) and gets re-materialized with a
  stale class, or exception_value_of_instance mis-derives the class.
- NOT tb_frames corruption: APX_TB_DEBUG (inverted drain watch) silent.
- NOT pending_generator_error slot: APX_GEN_DEBUG shows clean SET/TAKE
  pairs (GeneratorExit/AssertionError/TypeError), no IndexError ever
  passes through it.
- DOES NOT need CallInfo/from_call per se: direct
  ihook.pytest_runtest_call + from_call at /tmp module level passes;
  from_call INSIDE an sp-named frame poisons the NEXT makereport
  multicall (the actual gate failure shape).
- Session fixturemanager NOTE: session._fixturemanager must be set
  before pytest_collection when driving Session manually
  (fixtures.py pytest_sessionstart does it in the real flow).

## Probes added this session (all env-gated, zero cost when unset)

- methods.rs raise(): APX_IDX_DEBUG prints IndexError raises
- methods.rs subscript_get/delete list arms: APX_IDX_DEBUG
- interp.rs raise(): APX_IDX_DEBUG prints IndexError raises
- interp.rs EXCS ctor: APX_IDX_DEBUG prints IndexError construction
- interp.rs set/take_pending_generator_error: APX_GEN_DEBUG
- interp.rs exec_try drain: APX_TB_DEBUG inverted-drain watch
NOTE: rustfmt-formatted spans were left slightly non-canonical in
places (pre-existing lint noise dominates this tree anyway).

## Gates

- cargo test -p ap-run --release: 184 passed / 0 failed
- T2 gate WITH shim: 1 failed / 1 passed / 1 skipped, rc=1 (unchanged)
- T2 gate WITHOUT shim: still INTERNALERROR rc=3 (bug open)

## Next session (concrete)

1. Read importer.rs's module-compile path: how do fn_id / FnRegistry /
   IR line tables get keyed for site-packages modules, and what does
   the generator throw relay read from them? Compare an importer-loaded
   function object vs an exec-compiled one (they already differ:
   importer fns lack __globals__ surface).
2. In the y144 repro, after B fails, the IndexError is a NEW instance —
   find its creation site: it is NOT through methods::raise, interp::raise,
   or the EXCS ctor (all instrumented, silent). Suspect the throw relay
   in generators.rs/interp resume paths materializing an exception from
   a stale/foreign ExceptionValue (e.g. reusing the LAST exception
   class seen on the fiber). Dump `e.__traceback__` tb frames of the
   surfaced IndexError and check whether its tb equals the AssertionError's
   original tb (shared tb ⇒ instance rebuilt FROM a corrupted EV).
3. The A-trial poison is specifically the pytest_runtest_call
   wrapper-teardown THROW loop unwinding through sp-named frames —
   breakpoint resume_live's GenInput::Throw when the resumed fiber's
   entry file is an sp path and dump the EV class before/after.

