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

## Current status (update this block when it changes)
- Plans 00+01 COMPLETE. Plan 02 nearly complete: 02.01 values, 02.02 executor,
  02.03/02.03b classes+metaclass, 02.04 GC (weakref surfaces, unittest gate,
  MMTk design doc docs/gc-mmtk-swap.md), 02.08-lite + 02.08-full p1 (25+
  native modules, method surfaces) ALL COMMITTED.
- 80 commits. rust 247/0, pytest 1272/0. Corpus 150/197 (76%): values 46/47,
  fstrings 34/35, stdlib 17/48, classes 27/33, exceptions 15/15, gc 9/17.
- unittest/pathlib/enum/weakref IMPORTS ALL WORK (vendored stdlib chain).
- BUGHUNT 2026-10-17: 7 fixes landed; report in .hermes/audits/.
- PLAN 02 REMAINING: 02.05 tier-1 (split per risks) + 02.06 AoT + 02.07
  threading + 02.08-full p2 (re engine, complex, deep method surface).
- AFTER: Plan 03 WASM, Plan 04 packaging (uv) -> T1 scrapy attempt.
- Ecosystem targets locked (master.md tier table): scrapy T1, pytest T2,
  hermes-agent T3, fastapi T4, comfyui T5 (gates on Plan 10). TEST DATA ONLY -
  never published, no license constraints apply, comfyui included without
  restriction.
- pydantic-core (Rust extension) is the recurring blocker for T3/T4 - strongest
  single case for Plan 10 after basic C-API viability.
