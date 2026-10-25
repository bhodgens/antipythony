# RustPython donor policy (tiered, 2026-10-24)

RustPython (MIT, Rust) is a tiered donor, not a fallback-only one. Architecture
stays ours; leaf-level borrowing is approved with standard MIT attribution -
the same treatment as monty and ruff.

## Tier 1 - architecture: OURS (never adopted)

Parser/AST, compiler IR, eval loop, object model, GC, and import machinery are
ours to build. RustPython's architecture conflicts with locked decisions (ruff
parser, ap-ir, AOT/WasmGC goals), and its conformance ceiling is below our
byte-exact vs CPython 3.14 gate.

## Tier 2 - leaf-level adoption: APPROVED (MIT attribution required)

Four categories, all with standard MIT attribution (notice in
THIRD_PARTY_NOTICES; adapted files keep headers):

1. **Builtin function surfaces** - `vm/src/builtins/*` pure-Rust
   implementations (bytes.fromhex, os.utime, base64.urlsafe_b64encode, ...).
2. **Stdlib re-implementations** - Rust replacements for C-extension shims
   (hashlib, zlib stubs, select).
3. **Conformance corpus** - `python_tests/` mined for coverage gaps; tests are
   re-authored against our spec pack, no code coupling.
4. **Derive macros / slot patterns** - mechanical plumbing, port freely.

## Wasm constraint (binding for Plan 03)

Anything that must exist on the WasmGC target needs a pure-Rust
implementation - system libraries cannot exist in a Wasm sandbox. Native-only
system-lib backends (OpenSSL, CommonCrypto) may land later as opt-in
performance upgrades behind feature flags; they are never the base
implementation.
