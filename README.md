# ANTIPYTHONY

Pronounced like "antipathy" (/ænˈtɪpəθi/). An intentional homophone of the
English word "antipathy", not a misspelling of it.

A Python 3.14-compatible runtime and compiler, written in Rust.

Antipythony compiles Python source to two artifacts:

1. A single static native binary (Go-style UX: `antipythony build app.py -o app`).
2. A WasmGC module (portable, sandboxed).

Design choices that define the project:

- **Tracing GC, not refcounting.** Object destruction timing is a documented
  divergence from CPython (`__del__` runs; when is not guaranteed).
- **No GIL.** Python threads map to real OS threads.
- **Byte-exact differential conformance is the exit gate for every phase.**
  Every observable behavior is tested by running the same script under
  CPython 3.14 and under antipythony, then diffing stdout bytes, stderr
  bytes, and exit code. "Close enough" is a failure.
- **uv-based dependency resolution at build time** (planned, Plan 04).
- **No C extensions in v1.** An import that resolves to a C extension
  produces a compile-time error naming the module. C-API support is Plan 10.

License: Apache-2.0 WITH LLVM-exception. See `LICENSE`,
`THIRD_PARTY_NOTICES`, and `docs/attribution.md`.

## Prior art and borrowed design

**pon** (github.com/can1357/pon, Can Bölük) — antipythony's architecture
overlaps pon's substantially (ruff-pinned frontend, shared IR, Cranelift
tiered backend, differential conformance gating). Can Bölük granted this
project a full code-donor license (MIT/Apache-2.0 dual, adapt and vendor
freely with attribution) by email on 2026-10-16 — the primary evidence is
archived at `LICENSES/pon-license-grant-email.txt`, license texts in
`LICENSES/`. Current status: **designs and implementation strategies have
been studied from pon** (see `docs/pon-study-lowering.md` and
`docs/pon-study-tier1.md` — lowering shape, inline-cache/OSR/deopt
architecture); **no pon source code has been vendored into `crates/`
yet.** When adaptation lands, each derived file carries an attribution
header ("adapted from pon, Copyright Can Bölük, MIT/Apache-2.0") per
`plans/antipythony/master.md`, and pon stays cited in `docs/attribution.md`.
pon is not forked: antipythony diverges by design (WasmGC backend, MMTk
tracing GC, uv-integrated packaging, dual-backend IR contract).

Other cited sources (full detail in `docs/attribution.md` and
`LICENSES/`): monty (MIT) — ap-obj adaptation donor; ruff (MIT) — parser;
Cranelift/wasmtime; MMTk; CPython stdlib vendored files (PSF-2.0,
`vendored_stdlib/`); RustPython (MIT, fallback donor only); p2w and kohebi
specs (design reference only). Nuitka (AGPL) is a subprocess tool only —
never vendored or linked.

## Status (as of 2026-11-07, trunk `46729d4`)

The project is mid-build, executing a hierarchical plan tree
(`plans/antipythony/`). Current state:

| Plan | Scope | Status |
| --- | --- | --- |
| 00 Foundation | workspace scaffold, license gates, compat contract, spec pack, conformance harness | COMPLETE |
| 01 Frontend/IR | parser (ruff wrapper), IR, lowering, verifier | COMPLETE |
| 02 Native backend | object model, executor, classes, GC, stdlib modules, threading | ~90% LANDED: values, executor, classes+metaclass, GC, real-OS-thread threading (GIL v1), 25+ native stdlib modules, method surfaces; wave-3 memperf stack merged (g1-g6, `47be826`). REMAINING: 02.05 tier-1 JIT + 02.06 AoT (ap-codegen-native still a stub — the headline "single static binary" is the biggest untouched artifact), 02.08 method-surface tail, 02.09 no-GIL swap (green gate: T121 >=3x scaling), 02.10 scheduler (research only) |
| 03 WASM backend | ap-ir to WasmGC, wasmtime host | DESIGN + SKELETON IN TREE (docs/03-wasm-representation-design.md, spec, ap-codegen-wasm lower.rs + ap-host wasmtime runner @765ab61/0eff19f); leaves 03.02-03.07 implementation not started |
| 04 Packaging | uv integration, CLI, release/install | 04.01+04.02 COMMITTED (`6f0b886`: uv pure-wheel fetch + sys.path integration, ap-pack crate); 04.03 release eng / 04.04 wasm dist / 04.05 hybrid remain |
| 05 Ecosystem T2 | pytest gate, tier-2 fixtures | ACTIVE: T2 gate GREEN (see below); capsys teardown + traceback-precision polish in flight |
| 10 C extensions (stretch) | CPython C-API subset | NOT STARTED (gates T3/T4 on pydantic-core; case study docs/plan10-pydantic-core-study.md landed) |

What works today: parse to IR to verify to tier-0 execution. Generators
(PEP-342 lazy, real coroutine frames: async def bodies execute, await
binds, asyncio Task + run_until_complete runs end-to-end), closures,
exceptions with CPython-format tracebacks, classes with MRO and
metaclasses, f-strings, dict/list/str method surfaces, weakrefs, a
tracing collector, real OS threads (GIL v1 - interim stage, no-GIL swap
is leaf 02.09), and 25+ native stdlib modules (math, json, hashlib,
itertools, random, re with full backtracking engine, unittest runner,
pathlib, enum, dataclasses, ...) run byte-exact against CPython 3.14.
`import enum`/`unittest`/`inspect`/`dataclasses` no longer hang
(pydict self-alias deadlock fixed `3a80a4b`, 2026-11-07).

Real-software targets, current state:

- T1 scrapy 2.19.0: IMPORTS AND RUNS. A harness-driven file:// spider is
  byte-identical to CPython (docs/t1-gate.md). The real CrawlerProcess
  crawl COMPLETES END-TO-END (2026-11-05: Spider opened -> PARSE CALLED
  -> closed (finished), RC=0; NEXT-STEPS-W112.md rounds 20-26). Open
  blocker for the items.jsonl byte-compare: response.text crashes in
  w3lib/encoding.py resolve_encoding -> issue #13 (native multibyte
  codecs).
- T2 pytest 9.1.1: GATE GREEN - pytest.main runs a trivial 3-test suite
  to completion, verdict-identical with CPython (1 failed / 1 passed /
  1 skipped, rc=1), DEFAULT CONFIG (cacheprovider ON; full gate
  2026-11-04). Tier-2 fixtures pass: tmp_path, monkeypatch, generator
  fixtures, parametrize.

Conformance corpus: 274 case files under tests/corpus (pass-rate source
of truth: plans/**/PROGRESS.md; last recorded 233/271 ~= 86%, all
recorded failures pre-existing). Rust test suite: 61 suites / 444
passed / 0 failed / 1 ignored; full pytest tests/: 1977 passed,
656 xfailed, 93 xpass (stale ledger markers), exit 0 — measured
2026-11-07 @46729d4. Pre-Wasm pass bar (Amendment A-3): semantics must
be exact; byte-exactness divergences are tracked as GitHub issues, not
blocking.

The end goal is measured against real software, in tiers: run scrapy
(T1), pytest (T2), hermes-agent (T3), fastapi (T4), comfyui (T5).
T3/T4 are blocked on pydantic-core (Rust extension) - Plan 10 or a
pure-Python fallback. These targets are internal test data only, never
published. See `docs/path-to-targets.md`.

## Architecture

Rust workspace, one crate per layer. The dependency rule: backends depend
on `ap-ir`; nothing depends sideways, and the object-model ABI in `ap-obj`
is the only seam the two backends may implement.

```
crates/
  ap-parse/          ruff parser wrapper -> typed AST (owns the Python version pin)
  ap-ir/             compiler IR: types, ops, module/function bodies (no backends)
  ap-obj/            Python object model: values, types, dispatch, exceptions
  ap-gc/             GC: heap trait, collector, safepoints (MMTk swap design in docs/)
  ap-run/            tier-0 executor over ap-ir using ap-obj (the correctness baseline)
  ap-codegen-native/ ap-ir -> Cranelift CLIF -> JIT / static binary (stub; leaf 02.05/02.06)
  ap-codegen-wasm/   ap-ir -> WasmGC module (skeleton + lower.rs in tree; Plan 03 leaves not started)
  ap-host/           wasmtime host runner, WASI imports (skeleton in tree; Plan 03)
  ap-cli/            the `antipythony` binary: run/eval verbs (testability CLIs live here)
  ap-pack/           uv pure-wheel fetch + content-addressed site-packages (Plan 04.01/04.02 committed)
  ap-conform/        differential conformance runner (spawns CPython, diffs bytes)
  xtask/             build tasks: notices generation + drift check
```

Pipeline: `Python source -> ap-parse (AST) -> ap-ir (IR) -> verify ->
ap-run (tier-0) or ap-codegen-native (tier-1 JIT / AoT) or
ap-codegen-wasm`. Tier-1 must be invisible: any observable difference
between tier-0 and tier-1 output is a bug.

Key documents:

- `plans/antipythony/master.md` - the contract. Locked decisions, crate
  layout, license table, ecosystem tiers.
- `docs/compat-contract.md` - the CPython 3.14.0 compatibility contract:
  supported v1 subset, documented divergences, fail-loud rules.
- `docs/spec/` - the spec pack (S-NN-NN clause ids). The single source of
  truth for semantics. Test suites are written from it, never from
  implementation code.
- `tests/README.md` - corpus layout and the rules that keep goldens honest.
- `AGENTS.md` - operational state and durable conventions for
  agent-driven development on this repo.

## Development quick start

Requirements:

- Rust 1.94.0 (Homebrew toolchain; `rust-toolchain.toml` pins it for rustup
  environments).
- A real CPython 3.14 on PATH as `python3.14` (the reference interpreter
  for all differential tests; suites use `/opt/homebrew/bin/python3.14` on
  macOS).
- pytest (`pip install pytest`) for the conformance suite.

Build and test:

```
cargo build --workspace
cargo test --workspace          # Rust unit + differential integration tests
pytest tests/                   # full conformance suite (slow; per-suite is fine)
cargo run -p xtask -- notices --check   # THIRD_PARTY_NOTICES drift gate
cargo fmt --check
```

Run Python on the tier-0 executor:

```
cargo run -p ap-cli --bin ap-run-eval -- path/to/script.py
```

Testability CLIs (each suite drives one; see `tests/README.md`):

| Binary | Purpose |
| --- | --- |
| `ap-parse-check` | parse diagnostics (`crates/ap-parse/src/bin/ap_parse_check.rs`) |
| `ap-ir-dump` | parse + lower + print IR (`--verify` mode too) |
| `ap-verify-dump` | print one verifier diagnostic per line |
| `ap-run-eval` | execute a file at tier-0 (`file.py`) |
| `ap-obj-eval` | object-model expression evaluation |
| `ap-conform` | differential runner: `ap-conform run --corpus <cases> --impl <cmd>` |

## How correctness is enforced

1. The spec pack (`docs/spec/`) defines semantics with clause ids.
2. Every corpus case in `tests/corpus/<family>/` starts with
   `# Cites: <clause ids>`; `tests/corpus_traceability/` fails if any
   citation does not resolve.
3. Expected outputs are pinned bytes generated from live CPython
   (`expected/*.out|.err|.rc`). A corpus case is never weakened to pass a
   gate; environment-dependent goldens get regenerated with a documented
   correction note.
4. Implementation leaves and test leaves are written by different agents
   from the spec, never from implementation code.
5. Exit gates are per-leaf and listed in each plan's `orchestrator.md`.

## Repository layout

```
AGENTS.md                operational state + agent conventions (cross-session memory)
plans/antipythony/       the plan tree: master.md, per-plan orchestrators + PROGRESS.md
crates/                  the Rust workspace (12 crates, above)
docs/                    compat contract, spec pack, design notes, path-to-targets
tests/                   pytest suites (t01..t16) + differential corpus + lints
vendored_stdlib/         pure-Python stdlib files from CPython (PSF-2.0; NOTICE.md)
LICENSES/                license texts incl. vendored-stdlib notes
THIRD_PARTY_NOTICES      generated by `cargo run -p xtask -- notices`
deny.toml                cargo-deny allow-list config
.github/workflows/       CI gates (see docs/development.md)
```

## Further documentation

- `docs/development.md` - dev workflow, CI, testing the tests.
- `docs/architecture.md` - crate-by-crate detail and data flow.
- `docs/conformance.md` - the differential conformance methodology.
- `docs/compat-contract.md` - what is compatible, what diverges, and why.
- `docs/path-to-targets.md` - the T1-T5 ecosystem target dependency analysis.
