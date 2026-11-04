# Architecture

Antipythony compiles Python 3.14 source through a layered Rust pipeline
to two targets: native (JIT + ahead-of-time static binary) and WasmGC.
This document describes each layer and the rules that keep them clean.

## Data flow

```
Python source
    |
    v
ap-parse          typed AST (wraps the ruff parser; owns the version pin)
    |
    v
ap-ir             structured statement-tree IR: module/function bodies,
    |             ops, scope info. Verified here (IR verifier).
    +--> ap-run               tier-0: tree-walking/register executor
    +--> ap-codegen-native    tier-1: Cranelift CLIF; JIT + AoT (planned)
    +--> ap-codegen-wasm      WasmGC module emission (planned)
```

Both backends call the same object-model seam:

```
ap-obj            Python object model: values, types, dispatch, exceptions
    ^
ap-gc             heap behind trait Heap { alloc, gc_trigger, add_root_range }
```

## Crate reference

### ap-parse
Thin wrapper over the ruff parser crates (pinned 0.0.4 for MSRV).
Produces a typed AST and CPython-format parse diagnostics. This crate
owns the "which Python version are we" decision; nothing else hardcodes it.
Testability bins: `ap-parse-check`, `ap-ir-dump`, `ap-verify-dump`.

### ap-ir
The compiler IR: types, ops, module and function bodies. Deliberately
structured (stmt-tree), not a flat CFG; the tier-1 implementer may lower
hot functions internally, but the IR contract stays backend-free.
Rule: `ap-ir` depends on nothing above it. Backends depend on `ap-ir`;
never sideways.

### ap-obj
The Python object model, adapted from monty (MIT) with attribution.
Boxed value representation with small-int tagging, bigint via num-bigint,
CPython's latin1/UCS2/UCS4 string strategy, bytes, float with byte-exact
repr/str. Type objects, MRO (C3), descriptors, special-method dispatch,
metaclass hooks, the exception tree. The trait surface here is the ONLY
seam the native and WASM backends implement, so semantics stay identical
across targets.

### ap-gc
Tracing GC behind the `Heap` trait. Current implementation is a
hand-rolled collector (weakref surfaces, finalizer semantics where
`__del__` runs but timing is not guaranteed - documented divergence);
`docs/gc-mmtk-swap.md` holds the MMTk integration design. Containers
register trace roots; a collector mid-scan ABORTS on heap contention,
it never skips (a partial view sweeps live objects).

### ap-run
The tier-0 baseline executor: frames, fast/slow name lookup, control
flow, exceptions with CPython-format tracebacks, generators, context
managers. Also hosts the native builtin modules (builtins.rs,
builtins_modules.rs), the class machinery glue, the import machinery
(importer.rs: sys.modules cache, native registry, file finder over
sys.path, packages, circular-import partial visibility), and the native
`re` engine (sre.rs, a hand-rolled backtracking matcher; the Rust
`regex` crate was rejected because DFA semantics cannot produce
byte-exact backreference behavior). Every conformance case must pass
here before tier-1 is allowed to optimize it.

### ap-codegen-native (planned)
ap-ir to Cranelift CLIF. Tier-1 JIT: type-feedback specialization,
inline caches on attribute/call sites, OSR on hot loops, guards + deopt
back to tier-0 on assumption failure. Correctness rule: tier-1 is
invisible except by timing; the conformance suite runs both forced
tier-0 and adaptive modes. AoT path: cranelift-object, static link
(musl on linux), dead-stripped stdlib, target binary size <= 15MB.
Prior art: pon (dual MIT/Apache-2.0 to this project) study notes in
`docs/pon-study-tier1.md` and `docs/pon-study-lowering.md`.

### ap-codegen-wasm + ap-host (planned)
ap-ir to WasmGC module; wasmtime host runner with WASI imports. Parity
rule: a tier claim requires the corpus green in native AND wasm modes.

### ap-cli (planned)
The `antipythony` binary: `run`, `build`, `new` verbs. Go-style UX -
one command produces one static binary.

### ap-conform
The differential runner. For each corpus script: run under `python3.14`
and under the impl command, capture stdout+stderr+exit code, byte-diff,
emit a pass/fail table (machine-readable for CI). Supports `--filter`,
JSON output, and `--tier 0|adaptive` forwarding to the impl.

### xtask
Build tasks. `cargo run -p xtask -- notices` regenerates
THIRD_PARTY_NOTICES deterministically; `notices --check` exits 1 on
drift (the CI attribution gate).

## Vendored stdlib

`vendored_stdlib/` holds pure-Python stdlib files copied verbatim from
CPython (PSF-2.0, NOTICE.md with deviation log). Resolution order: the
native Rust registry wins over vendored files; only modules with no
native implementation are vendored. Vendoring exists because logic-heavy
modules (pathlib, dataclasses, typing, enum, unittest) would cost weeks
to rewrite natively and stress-test the interpreter harder than any
synthetic corpus.

## Threading model

No GIL. Per-object state is either thread-confined (proven by ownership
analysis) or behind atomic/sync wrappers in ap-obj. The `threading`
stdlib module maps to real OS threads. Gate: a scaling test must show
>= 3x on 4 cores.

## Cross-crate rules (from master.md)

1. `ap-ir` depends on nothing above it; backends depend on `ap-ir`, never
   sideways.
2. The `ap-obj` trait surface is the only seam backends may call.
3. All observable behavior gates go through `ap-conform`.
4. The spec pack (`docs/spec/`) is the single source of truth for
   semantics; test agents read spec + master, never implementation code.
5. Every implementation leaf names the test leaf(s) that gate it.
