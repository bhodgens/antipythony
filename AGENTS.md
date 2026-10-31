# ANTIPYTHANY - Project Context (auto-loaded by Hermes from this repo)

## What this project is
Python 3.14-compatible runtime and compiler, written in Rust. Compiles Python to
(1) single static native binaries and (2) WasmGC modules. Tracing GC (MMTk), no
GIL, uv-backed deps at build time. Byte-exact differential conformance vs CPython
3.14.0 is the exit gate for every phase. Working name: antipythany (repo dir:
~/git/anticoagulant).

## Where the truth lives
- Plan tree: `plans/antipythany/` - master.md is the contract; one orchestrator per
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
  study) - the 01.03 pre-task was skipped when no pon clone existed; dispatch a
  standalone pon-study task before Plan 02 dispatch.
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
- T1 SCRAPY CRAWL (in progress 2026-10-31, waves 90-109 in NEXT-STEPS.md):
  the wave-100..109 march took the crawl from "clean no-op" to RUNNING
  (exit 0, ~190s; extension ladder + engine ctor + downloader pass). The
  earlier "T1 DONE 2026-10-25" claim covered IMPORT + a harness-driven
  spider; the real CrawlerProcess crawl path is the current campaign.
  Current blocker (wave-109): twisted.conch chain imports cryptography
  (dispatched: v0 shim or eager-import bypass, deleg_ab35563b - RUNNING).
  GC issue #10 CLOSED (764ad5b): gc013 was stale (passes), gc009 was an
  unraisable source-echo indent bug (trim fix), gc015 was finalize-token
  alive attr shadowing (token exemption at the Opaque early-return);
  fence gc018-020 landed; gc family 20/20. Triage:
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
