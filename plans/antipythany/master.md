# ANTIPYTHANY - Master Plan

A Python 3.14-compatible runtime and compiler, written in Rust. Compiles Python to
native single static binaries (Go-style UX) and WasmGC modules (portable, sandboxed).
Tracing GC instead of refcounting. No GIL. uv-based dependency resolution at build
time. Byte-exact differential conformance against CPython is the exit gate for every
phase.

## Locked decisions (do not re-litigate in leaves)

| Decision | Value |
| --- | --- |
| Project name | antipythany |
| Language | Rust (workspace crates, see contracts below) |
| Backend order | Native first (Plan 02), then WASM (Plan 03) |
| C extensions v1 | None - extension imports fail with clear compile-time error. Rebuild-required C-API subset is v2 / Plan 10. |
| Dependencies | uv subprocess integration from v0 (Plan 04.01); stdlib-only until that lands |
| Artifacts | Two artifacts: native binary + separate .wasm. All three distribution plans are written: native (02), wasm (03), hybrid (04.05) |
| License | Apache-2.0 WITH LLVM-exception (whole project) |
| Python target | 3.14, pinned. One version at a time. |
| Leaf budget | 256K context per leaf agent |

## License and attribution contract

- Project license: `Apache-2.0 WITH LLVM-exception` (SPDX). Rationale: patent grant
  (Apache-2.0 section 3) plus the LLVM exception lets user-compiled binaries embed our
  runtime without carrying our NOTICE file. Mirror the Rust/Cranelift practice.
- Borrowed components and obligations:
  - ruff parser (MIT, Rust) - notice in THIRD_PARTY_NOTICES
  - Cranelift + wasmtime (Apache-2.0 WITH LLVM-exception) - notice + exception text
  - MMTk (Apache-2.0) - notice
  - pydantic/monty (MIT, Rust) - notice; adapted code keeps its headers
  - RustPython (MIT, Rust) - notice; fallback donor only
  - abilian/p2w (MIT, Python) - DESIGN REFERENCE ONLY (wrong language, own IR); no code
  - kohebi spec docs (Apache-2.0) - design reference
  - pon (LICENSE GRANTED by Can Bölük, me@can.ac - email correspondence, received
    2026-10). pon code MAY be studied and adapted per the granted terms. TODO before
    first pon-derived commit: (1) record the exact license text/terms in
    LICENSES/pon-License (or confirm it is MIT/Apache so THIRD_PARTY_NOTICES covers
    it), (2) cargo-deny allowlist entry if it is a bespoke grant, (3) attribution
    header on any adapted file. If the grant is private/use-only rather than an OSS
    license, pon-derived code must be isolated in contrib/pon-derived/ and kept out
    of redistributable artifacts until terms allow.
  - Nuitka (AGPL-3.0) - external subprocess tool only, never vendored or linked
- CI gate: `cargo-deny` + `cargo-about` generate `THIRD_PARTY_NOTICES`; release build
  fails if the file is stale. Leaf 00.04.
- Every crate header carries the SPDX line and the generated-notices note.

## Ecosystem compatibility targets (objectives, not v1 gates)

Ordered milestones. Each target program must run CORRECTLY (byte-fair outputs,
exit codes) before its tier is claimed. Corpus cases derived from each target's
real source patterns land in tests/corpus per tier; they gate the tier, not v1.

| Tier | Target | What it stresses | Blocker |
| --- | --- | --- | --- |
| T1 | scrapy (core spider run, no lxml) | generators, contextlib, selectors, deferreds-style async, dynamic imports | none - pure-Python path exists |
| T2 | pytest (collect+run a small suite) | inspect/frames/code objects, assertion rewriting (AST manipulation), import modes, plugins | none but deep introspection fidelity required |
| T3 | hermes-agent (chat turn, httpx+rich path) | pydantic OR pydantic-free path, async, decorators, dataclasses | pydantic-core is a Rust extension - gates on Plan 10 or a pure-Py pydantic fallback |
| T4 | fastapi (hello-world app + TestClient) | runtime annotation inspection, pydantic-core, asyncio, ASGI | same pydantic-core gate as T3 |
| T5 | comfyui (load + run a simple workflow) | torch and friends - the heaviest binary-wheel stack in the list | Plan 10 full C-extension support; this is the hail mary made concrete |

Rules: tiers are independent (T1 does not depend on anything above). A tier claim
requires its corpus slice green in native AND wasm modes. These targets are
INTERNAL TEST DATA ONLY - validation that antipythany runs real software. Nothing
from any target is published or shipped in product code; target repos are fetched
as fixtures, run, and discarded. No license constraint applies to that use:
vendor excerpts are never redistributed (this includes comfyui GPL-3.0 - no
constraint under test-data-only use; the earlier GPL handling note is void).

## Interface contracts between ALL children

Crate layout (Rust workspace, root `Cargo.toml`):

```
crates/
  ap-parse/         # ruff wrapper -> typed AST (owns Python version pin)
  ap-ir/            # compiler IR: types, ops, module/function bodies (no backends)
  ap-obj/           # Python object model: values, types, dispatch, exceptions
  ap-gc/            # MMTk binding + heap policies behind trait Heap
  ap-run/           # tier-0 baseline executor over ap-ir using ap-obj
  ap-codegen-native/# ap-ir -> Cranelift CLIF -> object files / JIT
  ap-codegen-wasm/  # ap-ir -> WasmGC module
  ap-host/          # wasmtime host runner, WASI imports, embedding API
  ap-cli/           # `antipythany` binary: run/build/new verbs
  ap-conform/       # differential conformance runner (spawns CPython)
```

Cross-crate rules:

1. `ap-ir` depends on nothing above it. Backends depend on `ap-ir`; never sideways.
2. The object model ABI (`ap-obj` traits) is the only seam backends may call. Native
   and WASM implement the same trait surface so semantics stay identical.
3. All observable behavior gates go through `ap-conform`: run script under CPython and
   antipythany, diff bytes on stdout/stderr and exit code.
4. The spec pack (00.05) is the single source of truth for SEMANTICS. Test agents read
   spec pack + this master only. They never read implementation code.
5. Every leaf lists the test leaf(s) in `tests/` that gate its completion.

## Test tree (separate pass, separate agents)

Lives at `plans/antipythany/tests/`. Protocol:

- Test documents are written ONLY from the spec pack (00.05) and master contracts.
- Test agents are different agents, dispatched in a separate pass - never the agent
  that implemented the leaf under test.
- Test code lives in the repo under `tests/` (integration/e2e) and `crates/*/tests/`
  for unit tests, with the unit-test files authored from spec, not from impl.
- Each implementation leaf's completion gate = its named test leaf green, run by the
  orchestrator after a DIFFERENT agent wrote/passed the tests.

## Child index and dispatch order

| Plan | Directory | Depends on | Status |
| --- | --- | --- | --- |
| 00 Foundation | `00-foundation-contract/` | none | NOT STARTED |
| 01 Frontend/IR | `01-frontend-ir/` | 00 | NOT STARTED |
| 02 Native backend | `02-native-backend/` | 01 | NOT STARTED |
| 03 WASM backend | `03-wasm-backend/` | 01, 02 (tier-0 semantics proven) | NOT STARTED |
| 04 Packaging | `04-packaging-ecosystem/` | 02 (native verb first), 03 for wasm verbs | NOT STARTED |
| 10 C extensions (HAIL MARY, isolated) | `10-c-extensions-stretch/` | nothing blocks on it | NOT STARTED |
| T Test tree | `tests/` | spec pack from 00.05 | NOT STARTED |

Dispatch: 00 -> (01 with T spec leaves in parallel) -> 02 -> 03 -> 04. Plan 10 may
start its research leaf any time after 02 exists; its failure must not block 0-4.

## Completion tracking

Updated by orchestrator after each leaf review.

| Leaf | Status |
| --- | --- |
| 00.01 | COMPLETE (d01d477) |
| 00.02 | COMPLETE (24c97ed) |
| 00.03 | COMPLETE (0f18715) |
| 00.04 | COMPLETE (0f18715) |
| 00.05 | COMPLETE (e703623/80b3d88); amendments A-1..A-6 applied (3f1bb33) |
| T-track | T-01..T-11, T-14..T-16 AUTHORED; T-12/T-13 BLOCKED on spec 07. See tests/TRACKING.md |

## Integration review plan

After each plan completes: main model (not a subagent) reviews cross-crate seams:
- ap-ir purity (no backend imports leaked)
- ap-obj trait surface unchanged across native and wasm consumers
- conformance corpus grows monotonically; no corpus test deleted to make a gate pass
- THIRD_PARTY_NOTICES regenerated and diffed
