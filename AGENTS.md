# ANTIPYTHONY - Project Context (auto-loaded by Hermes from this repo)

## What this project is
Python 3.14-compatible runtime and compiler, written in Rust. Compiles Python to
(1) single static native binaries and (2) WasmGC modules. Tracing GC (MMTk), no
GIL, uv-backed deps at build time. Byte-exact differential conformance vs CPython
3.14.0 is the exit gate for every phase. Working name: antipythony (repo dir:
~/git/anticoagulant).

## Where the truth lives
- Plan tree: `plans/antipythony/` - master.md is the contract; one orchestrator per
  plan (00 foundation, 01 frontend/IR, 02 native, 03 wasm, 04 packaging,
  10 C-extensions hail mary); tests/README.md is the separate-authorship test track.
- Locked decisions are in master.md. Do not re-litigate them in leaves.
- This file: operational state + durable conventions only.

## License state
- Project: Apache-2.0 WITH LLVM-exception.
- pon (can1357/Can Bölük, me@can.ac): DUAL LICENSE MIT/Apache-2.0 GRANTED via
  email, 2026-10; confirmed by user - full code donor, adapt/vendor freely with
  attribution. BEFORE first pon-derived commit: (1) LICENSES/ entries (standard
  MIT + Apache texts), (2) attribution headers on adapted files. Levers
  01.03 (pon-ir lowering study) and 02.05 (pon tier-1 inline-cache/OSR/deopt
  study) - DONE: both study docs landed (docs/pon-study-lowering.md +
  docs/pon-study-tier1.md @ 1237c75, clone d0a1717, ongoing updates
  7238cc5). LICENSES/ entries + attribution headers still required
  before first pon-DERIVED CODE commit (studies cite, don't vendor).
- Nuitka: AGPL - subprocess tool only, never vendor/link.
- p2w: design reference only (wrong language).

## Execution protocol (binding)
- Leaves fit one 256K-context agent. Test leaves are written from the spec pack
  (docs/spec/), by different agents, in a separate pass - never from impl code.
- Corpus only grows. Never weaken a test to pass a gate.
- Orchestrator (main model) reviews in-session; parents determine completion.
- "Commit then implement" / "fix all" = pre-authorized per user convention.
- Specs before code: nothing dispatches from Plans 01+ until 00.05 spec pack lands.
- Repo gates run in CI, NOT git hooks. This machine's global
  `core.hooksPath=~/.githooks` (LFS hooks only - verified: `git config
  --get core.hooksPath`) silently disables repo-local hooks, so any future
  repo hook is a no-op here. If hooks are ever wanted, onboarding must set
  `git config core.hooksPath .githooks` per-repo. See docs/development.md.

## Current status (update this block when it changes)
- Plans 00+01 COMPLETE. Plan 02 nearly complete: 02.01 values, 02.02 executor,
  02.03/02.03b classes+metaclass, 02.04 GC (weakref surfaces, unittest gate,
  MMTk design doc docs/gc-mmtk-swap.md), 02.08-lite + 02.08-full p1 (25+
  native modules, method surfaces) ALL COMMITTED.
- unittest/pathlib/enum/weakref IMPORTS ALL WORK (vendored stdlib chain).
- BUGHUNT 2026-10-17: 7 fixes landed; report in .hermes/audits/.
- PLAN 02 REMAINING: 02.05 tier-1 (split per risks) + 02.06 AoT + 02.08
  method surfaces + 02.09 no-GIL swap (NEW gated leaf - T121 >=3x scaling
  is its GREEN gate per master.md Design intent; dispatchable independent
  of 02.05/06) + 02.10 Go-style scheduler (stretch, research-only for now).
- ROBUSTNESS-CI CAMPAIGN: plan tree at `plans/robustness-ci/` (master.md +
  7 branches, 24 leaves) implements ALL findings of
  .hermes/audits/robustness-review-2026-10-19.md (CI core, test integrity,
  lint gates, provenance, fuzzing, CLI/docs, gated decomposition). Status:
  WAVE A DISPATCHED 2026-10-19 (00.01, 00.02, 02.02, 03.03). Branch 06
  hard-gated on Plan 02 waves.
- Status block verified at 5f1e132 2026-10-19 by leaf 03.03 (fresh
  re-measure): 94 commits; `cargo test --workspace` 289 passed / 0 failed
  / 1 ignored; `xtask notices --check` exit 0 (151 deps, up to date);
  corpus inventory 8 families / 270 .py cases / 696 expected files
  (pass-rate counts: see plans/**/PROGRESS.md, source of truth). Previous
  block (stale): "80 commits. rust 247/0, pytest 1272/0. Corpus 150/197".
  CAVEAT: measured with sibling 02.07 threading work uncommitted in
  crates/ap-run/src (threading.rs / threading_call.rs in flight); rust
  gate green at that state, full `pytest tests/ -q` still running at
  verify time (~50% done: 0 F/0 E, 3 skipped, 280 xfail, 0 xpass;
  differential ap-conform suites are the long pole).
- T1 SCRAPY: DONE 2026-10-25. scrapy 2.19.0 imports and runs; file:// spider
  items.jsonl byte-identical vs CPython (verified). Gate doc: docs/t1-gate.md.
  Chain: a14fe0d (sys.path), 9fe23b4 (lxml shim + interpreter wave), a021802
  (lxml 24/24), cfe2465 (A-1 RustPython adaptations). ~25 blockers cleared.
  Known residual Class-X divergences tracked as GitHub issues #2-#5
  (byte-divergence label) per Amendment A-3/D6 (86cefd8): pre-Wasm standard is
  works-best-effort; byte-exact returns post-Plan-03. A-1 RustPython wave 1
  landed: errno 108 pairs, binascii qp/uu/hqx, bytes hex sep, os pipe/kill/
  stat/utime arms, ClosureMarker fix, function-not-iterable fix (cfe2465).
  Corpus 149/178=84%+; stdlib 28/48.
- T2 PYTEST (gate GREEN 2026-10-28, waves 6-23 + 25-57 in NEXT-STEPS.md):
  PEP-342 lazy generators (7c0a645) cleared the pluggy-wrapper blocker
  (issue #7 CLOSED 2026-10-31). T2 minimal gate: pytest.main runs a 3-test
  trivial suite to completion, verdict-identical with CPython (1 failed /
  1 passed / 1 skipped, rc=1). Tier-2 fixtures pass: tmp_path, monkeypatch,
  generator fixtures, parametrize. Key fixes: ev.instance preservation
  (5424f1b), per-exception tracebacks with per-frame FILE attribution
  (98b2964), stream-base subclasses (CaptureIO), fd-backed os primitives,
  builtin method-surface type gating (list.keys no longer exists),
  tuple.index start/stop, union __args__/__origin__/isinstance surfaces.
  Remaining Tier-2 polish: capsys fixture teardown int-None (tracked in
  NEXT-STEPS waves 50-57), traceback frame linenos imprecise (degrades).
- T2 PYTEST FULL GATE (GREEN 2026-11-04, W112 campaign ee4239e..a6b84ac):
  DEFAULT-CONFIG gate (cacheprovider ENABLED, no -p no:cacheprovider)
  verdict-identical with CPython: 1 failed / 1 passed / 1 skipped,
  MAIN_RC=1, zero INTERNALERROR (t2g104/t2g105/t2g106). The interim
  -p no:cacheprovider config is RETIRED. Root causes fixed: iter.rs
  swallowed generator-body errors as exhaustion (degenerate tracebacks
  -> _rawentry/ishidden crashes); gen.throw dropped instance __ap_tb__;
  os.umask returned None instead of the previous mask (cacheprovider
  _make_cachedir 0o777 - umask); open('wb') was a text stream;
  shutil.rmtree was a v1 stub. (Was: compile(PyCF_ONLY_AST) a name-surface
  stub - now a REAL tree since the W114 AST campaign; see the T2
  REGRESSION item below.) Known residual: the staged-source degradation
  path may interact with real AST trees (see T2 regression). GOTCHA: the
  rename left STALE test binaries with pre-rename baked paths
  (target/debug/antipythony, harness_selfcheck, xtask_gates) - if
  gates fail with missing vendored_stdlib/NotFound, touch the test
  files and rebuild before debugging.
- T1 SCRAPY CRAWL (in progress; campaign doc NEXT-STEPS-W112.md rounds
  20-26): CRAWL COMPLETES END-TO-END 2026-11-05 (bridge-e6.py: Spider
  opened -> PARSE CALLED -> Spider closed (finished) -> HARNESS-DONE
  RC=0). Root fixes a7ff45b: async-for over user async-iterators
  (Stmt::For async arm + aiter/anext protocol), fiber super-context
  relay (pending_super_ctx rode GenFrame/GenState through resume_live -
  zero-arg super() inside async-gen methods worked again), staged
  scrapy LocalWeakReferencedCache __contains__ shim (membership fell
  to __getitem__ sequence scan -> infinite loop). Bad-yield bridge:
  vendored tasks.py Task.__step recognizes twisted Deferred ->
  asFuture(loop) (leaf-verified: CPython raises the same 'bad yield'
  on raw-Deferred-await; ours now bridges). PREMISS CORRECTION:
  "install_reactor HANGS" (HANDOFF-2026-11-03) was NOT a hang - the
  scrapy+twisted import chain legitimately takes 70-120s in DEBUG
  builds (~270 modules re-parsed per process); probes need timeout
  >=300s. Current blocker (leaf running): response.text crashes in
  w3lib/encoding.py resolve_encoding -> blocks the items.jsonl
  byte-compare gate (docs/t1-gate.md standard).
- T2 REGRESSION OPEN 2026-11-05 (leaf running): AST campaign commits
  125f205..cba6036 (compile(PyCF_ONLY_AST) real tree, walk parity vs
  CPython 3.14) broke the default-config T2 gate: INTERNALERROR
  IndexError stack_summary[0] at _pytest/_code/code.py:223
  get_python_framesummary (leaf's staged source.py full-range
  degradation no longer fires now that real AST flows). Fix must keep
  AST parity (W114 scorecard green) - do NOT revert ast_bridge.
- T1 CRAWL history (waves 90-110, pre-round-20 state - superseded by
  the block above, kept for fix provenance):
  W110-5 LANDED (85432c9): real coroutine frames - async def bodies
  execute, send() runs to completion with true StopIteration values,
  await binds (PEP-492 drive loop with gen_yield relay), async-gen
  protocol surface, iscoroutine co_flags. En-route fixes:
  time.monotonic/perf_counter were ~always 0.0 (all call_later timers
  frozen), select.poll multiplied ms timeouts by 1000, CPython
  private-name mangling (__x -> _Cls__x aliases), sys.is_finalizing.
  Follow-ups (d7b827d): membership over generators/iterators,
  warnings.py prefers _py_warnings hook protocol (catch_warnings
  records now work). Chain: spider opens, Scheduler/RFPDupeFilter
  construct, DL resumes, exit 0. Findings: docs/wave-110-crawl.md.
  GC issue #10 CLOSED (764ad5b):
  gc013 was stale (passes), gc009 was an unraisable source-echo
  indent bug (trim fix), gc015 was finalize-token alive attr
  shadowing (token exemption at the Opaque early-return); fence
  gc018-020 landed; gc family 20/20. Triage:
  docs/gc-issue10-triage.md.
  Wave-110 salvage committed 98b2964: T2 union surfaces, per-frame tb
  file attribution, scrapy shims (urllib proxy/service_identity/
  tldextract/OpenSSL TLSv1/inspect degrade reorder); gates 417/0, corpus
  233/271 (all fails pre-existing; T104-event-handshake FIXED).
- AFTER: Plan 03 WASM, Plan 04 packaging (uv).
- Ecosystem targets locked (master.md tier table): scrapy T1, pytest T2,
  hermes-agent T3, fastapi T4, comfyui T5 (gates on Plan 10). TEST DATA ONLY -
  never published, no license constraints apply, comfyui included without
  restriction.
- pydantic-core (Rust extension) is the recurring blocker for T3/T4 - strongest
  single case for Plan 10 after basic C-API viability.
