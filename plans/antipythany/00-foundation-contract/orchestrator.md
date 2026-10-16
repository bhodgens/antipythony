# Plan 00 - Foundation and Contract

Goal: repo scaffold, license/attribution gates, the compatibility contract, the
conformance harness skeleton, and the spec pack that test agents will read.

## Design

Repo layout:

```
antipythany/
  Cargo.toml            # workspace
  LICENSE               # Apache-2.0 WITH LLVM-exception full text
  THIRD_PARTY_NOTICES   # generated, committed
  deny.toml             # cargo-deny config
  crates/               # see master contract
  docs/spec/            # the spec pack (00.05)
  tests/                # e2e conformance corpus + integration tests
  plans/                # this plan tree (do not ship)
```

## Leaves

### 00.01 - Workspace scaffold
Create the workspace: root Cargo.toml with all ten crates from the master contract as
empty members (each with Cargo.toml, src/lib.rs, `#![deny(missing_docs)]` off for
now), rust-toolchain.toml pinned, rustfmt.toml, clippy.toml, .gitignore, LICENSE
(Apache-2.0 WITH LLVM-exception text - copy from
https://spdx.org/licenses/LLVM-exception.html + standard Apache-2.0 file),
`cargo build` green.
Expose: workspace member names and paths (frozen - leaves in 01-04 use these).
Gate: `cargo build --workspace` exits 0.

### 00.02 - Attribution gate
deny.toml (licenses allow MIT/Apache-2.0/BSD/Unicode/ISC/Zlib, reject AGPL/GPL except
documented dev-only exceptions), cargo-about template for THIRD_PARTY_NOTICES, CI
recipe (justfile or xtask) that fails when notices are stale. Add ruff as a
dependency of ap-parse (MIT) to prove the pipeline works end to end.
Expose: `xtask notices` regenerates THIRD_PARTY_NOTICES deterministically.
Gate: deliberate stale-notice dry run fails; regenerated run passes.

### 00.03 - Compatibility contract doc
`docs/compat-contract.md`: CPython 3.14.0 reference; supported subset v1 (pure Python
+ stdlib modules that need no C extension); fail-loud rule: any import resolving to a
C extension module produces a compile-time diagnostic naming the module and pointing
to the extension policy doc; integer semantics (arbitrary precision, no overflow);
destruction timing (GC - `__del__` timing is NOT guaranteed, documented divergence);
float repr byte-exactness requirement; sys.exit/stderr/exit-code contract.
Expose: this doc is input to the spec pack; leaf 10.x research may relax v2 clauses
only by PR against this file.

### 00.04 - Conformance harness skeleton (ap-conform)
CLI: `ap-conform run --corpus tests/corpus/*.py --impl <cmd>`. For each corpus script:
run under `python3.14` and under the impl command, capture stdout+stderr+exit code,
byte-diff, emit pass/fail table (one line per case, machine-readable mode for CI).
Reference impl invocation is `python3.14` from PATH (documented requirement).
Expose: ap-conform crate with the runner; a `--filter` flag; JSON output mode.
Gate: harness runs today with `--impl python3.14` (self-comparison) and 2 trivial
corpus scripts, all green.

### 00.05 - Spec pack (THE test-authoring input)
`docs/spec/` - self-contained semantic specs, written from the compat contract and
CPython 3.14 docs, NOT from any implementation:
- 01-values-and-objects.md (numbers, strings, containers, truthiness, equality vs
  identity, hashing, repr/str)
- 02-control-flow.md (scoping, closures, generators, exceptions incl. chaining and
  traceback text format)
- 03-classes.md (attribute lookup order, descriptors, MRO, metaclass hook points)
- 04-modules-imports.md (import resolution, module namespace, main vs imported)
- 05-builtins-stdlib-v1.md (the exact builtins + stdlib module list for v1)
- 06-observable-behavior.md (stdout/stderr/exit codes, what ap-conform diffs)
Each doc: normative MUST/SHOULD statements with numbered clauses (S-xx ids). Test
leaves cite clause ids. This is the only semantics source test agents receive.
Gate: main model review - every v1 conformance corpus case (00.04) must be traceable
to at least one clause id; spot-check 10 clauses against real CPython behavior.

## Dependencies
None. This plan is first.

## Dispatch notes
00.01 -> 00.02 -> (00.03 + 00.04 parallel) -> 00.05. Spec pack (00.05) unblocks the
entire test tree; treat its completion as the highest priority after 00.01.
