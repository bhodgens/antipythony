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

## Next session (concrete) — UPDATED 2026-11-04 after live debugging

The prior session's step 2 was executed with the FRESH binary (the
earlier "no instrumented site fires" finding was a stale-binary artifact).
NEW FACTS:

- The B-surfaced IndexError comes from _pytest/_code/code.py:1170
  `last = traceback[-1]` (FormattedExcinfo.repr_excinfo) — `traceback`
  is a list built by StackSummary.extract from the exception's tb chain.
- The thrown AssertionError's tb chain EXISTS (5 frames, driver frames,
  verified by walking tb_next manually in the repro except-handler) —
  the relay does NOT lose the exception or its tb object.
- StackSummary.extract produces an EMPTY list from that tb chain →
  [-1] IndexError. Prime suspect: _get_code_position()
  (traceback.py:424-428) — `code.co_positions()` on our synthetic code
  object + islice(lasti//2, None) + next() → if co_positions yields
  fewer entries than lasti//2 (or the gen is empty when tb_lasti is
  stale/large), next() raises StopIteration inside the extract walk;
  extract silently drops such frames → 0 frames → empty list.
- Secondary symptom explained: traceback.print_exc() printed NOTHING in
  the handler — same extract-yields-nothing defect.
- Trial A is CLEAN: call.excinfo.value is AssertionError (args=()).
  The corruption is purely in tb→summary extraction after the throw.

## Live debugging round 3 (2026-11-04, this session) — tb-carry fixes landed, relay hop still open

Fixes COMMITTED in this round (interp.rs/methods.rs):
1. raise_from_value ObjKind::Exception arm: re-raising a materialized
   instance now routes through exception_value_of_instance (preserves
   identity + __ap_tb__ carry-back) instead of building a fresh EV with
   empty tb.
2. exception_value_of_instance: carries the instance's recorded
   __ap_tb__ entries back into ev.tb (was: dropped on every rebuild).
3. exception_instance: writes __ap_tb__ UNCONDITIONALLY (was skipped on
   empty tb), so once any hop has entries, continuity holds.
4. raise_from_value: when ev.tb is STILL empty, derives entries from the
   instance's live __traceback__ chain (tb_next walk over tb_lineno /
   f_code.co_filename / co_name; guard 64 frames).
5. methods.rs subscript_get: backtrace + pyloc debug under APX_IDX_DEBUG.

NEW FACTS from round 3:
- The prior "no instrumented site fires" finding was a STALE-BINARY
  artifact. With a fresh binary, methods::raise DOES fire:
  `list len=0 idx=-1` — a Python-level [-1] on an EMPTY list.
- The B-surfaced IndexError's origin: _pytest/_code/code.py:1170
  `last = traceback[-1]` in repr_traceback AFTER filter_excinfo_traceback
  produced an empty Traceback for the relayed AssertionError.
- Trial A's AssertionError instance carries NO __ap_tb__ (probe8) and
  its __traceback__ renders the 1-frame fallback (probe10: driver line 1).
  exc.tb was empty at the from_call-catch materialization — the 5-entry
  rendering in excinfo.traceback comes from the excinfo's OWN retained
  EV.tb copy (5 entries captured earlier), NOT from the instance attr.
- So tb continuity breaks at the FIRST exception_instance call for the
  AssertionError: exc.tb is empty at that point. The 5 entries existed
  in the EV DURING propagation (frame-exit hooks insert wrapper frames)
  but were gone by the from_call catch. The loss site is the hop(s)
  between the wrapper teardown catches and the from_call handler — most
  likely the fiber-crossing at pending_generator_error / resume_live
  ret, or a wrapper teardown that rebuilds the EV. Next probe: APX_TBDBG
  (APTB lines) around trial A + instrument resume_live ret for
  GenInput::Throw outcomes printing ev.tb.len().

Verdict: NOT FIXED yet; shim still required. The tb-carry mechanism is
now correct end-to-end; the remaining hole is the single hop that
empties ev.tb before from_call's catch. Next session: find that hop.

Live probe results (walk_tb / StackSummary / co_positions all verified
in-repro): walk_tb yields all 5 frames with correct (frame, lineno);
positions are (None,None,None,None) — handled; StackSummary.extract
returns 5 entries. The tb→summary extraction is NOT the defect.
ALSO: trial A's excinfo is CORRECT (AssertionError, args=()).

REFINED: the corruption appears only in the real makereport flow's
repr path: repr_traceback(code.py:1155, ExceptionInfoFormatter in this
staged tree) → filter_excinfo_traceback → if the FILTERED Traceback is
EMPTY → code.py:1170 `last = traceback[-1]` → our exact IndexError.
So the thrown-in AssertionError's ExceptionInfo (built from the wrapper
teardown relay) carries a tb whose entries are ALL hidden by
TracebackEntry.ishidden (path under _pytest/* + is_pytest internals
check), OR the tb_entries list is empty in the EV→instance rebuild.
Next probe: after B fails, build ExceptionInfo.from_exception on the
relayed error inside the makereport wrapper (not at driver level) and
print per-entry path/ishidden — driver-level from_exception shows
1 non-hidden entry (verified), so the divergence is in the relay-built
excinfo, not the driver-level one.

SIDE FINDING: staged _pytest/fixtures.py carried stale trace-probe
scaffolding (TV/PRUNE/FILL/CFI loggers to /tmp/t2coll.log) — removed,
pristine restored, kept only the legitimate finalizers-leak assert
message. T2 gate re-verified green after cleanup.

