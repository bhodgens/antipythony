# ANTIPYTHANY

A Python 3.14-compatible runtime and compiler, written in Rust.

Antipythany compiles Python source to two artifacts:

1. A single static native binary (Go-style UX: `antipythany build app.py -o app`).
2. A WasmGC module (portable, sandboxed).

Design choices that define the project:

- **Tracing GC, not refcounting.** Object destruction timing is a documented
  divergence from CPython (`__del__` runs; when is not guaranteed).
- **No GIL.** Python threads map to real OS threads.
- **Byte-exact differential conformance is the exit gate for every phase.**
  Every observable behavior is tested by running the same script under
  CPython 3.14 and under antipythany, then diffing stdout bytes, stderr
  bytes, and exit code. "Close enough" is a failure.
- **uv-based dependency resolution at build time** (planned, Plan 04).
- **No C extensions in v1.** An import that resolves to a C extension
  produces a compile-time error naming the module. C-API support is Plan 10.

License: Apache-2.0 WITH LLVM-exception. See `LICENSE`,
`THIRD_PARTY_NOTICES`, and `docs/attribution.md`.

## Status (2026-10)

The project is mid-build, executing a hierarchical plan tree
(`plans/antipythany/`). Current state:

| Plan | Scope | Status |
| --- | --- | --- |
| 00 Foundation | workspace scaffold, license gates, compat contract, spec pack, conformance harness | COMPLETE |
| 01 Frontend/IR | parser (ruff wrapper), IR, lowering, verifier | COMPLETE |
| 02 Native backend | object model, executor, classes, GC, stdlib modules, tier-1 JIT, AoT, threading | ~85% (tier-1 JIT, AoT, threading remain) |
| 03 WASM backend | ap-ir to WasmGC, wasmtime host | NOT STARTED |
| 04 Packaging | uv integration, CLI, release/install | NOT STARTED |
| 10 C extensions (stretch) | CPython C-API subset | NOT STARTED |

What works today: parse to IR to verify to tier-0 execution. Generators,
closures, exceptions with CPython-format tracebacks, classes with MRO and
metaclasses, f-strings, dict/list/str method surfaces, weakrefs, a tracing
collector, and ~25 native stdlib modules (math, json, hashlib, itertools,
random, re, unittest chain, pathlib, enum, dataclasses, ...) run
byte-exact against CPython 3.14.

Conformance corpus: 150/197 cases green (76%). Rust test suite: 289/0.
pytest suite: 1272/0.

The end goal is measured against real software, in tiers: run scrapy (T1),
pytest (T2), hermes-agent (T3), fastapi (T4), comfyui (T5). These targets
are internal test data only, never published. See
`docs/path-to-targets.md`.

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
  ap-codegen-native/ ap-ir -> Cranelift CLIF -> JIT / static binary (planned)
  ap-codegen-wasm/   ap-ir -> WasmGC module (planned)
  ap-host/           wasmtime host runner, WASI imports (planned)
  ap-cli/            the `antipythany` binary: run/build/new verbs (planned)
  ap-conform/        differential conformance runner (spawns CPython, diffs bytes)
  xtask/             build tasks: notices generation + drift check
```

Pipeline: `Python source -> ap-parse (AST) -> ap-ir (IR) -> verify ->
ap-run (tier-0) or ap-codegen-native (tier-1 JIT / AoT) or
ap-codegen-wasm`. Tier-1 must be invisible: any observable difference
between tier-0 and tier-1 output is a bug.

Key documents:

- `plans/antipythany/master.md` - the contract. Locked decisions, crate
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
| `ap-parse-check` | parse diagnostics |
| `ap-ir-dump` | parse + lower + print IR (`--verify` mode too) |
| `ap-verify-dump` | print one verifier diagnostic per line |
| `ap-run-eval` | execute a file at tier-0 (`[--tier 0] file.py`) |
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
plans/antipythany/       the plan tree: master.md, per-plan orchestrators + PROGRESS.md
crates/                  the Rust workspace (11 crates, above)
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
