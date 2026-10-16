# Plan 03 - WasmGC Backend

Goal: the same ap-ir, lowered to a WasmGC module. Portable, sandboxed artifact.
Builds ONLY after Plan 02 proves semantics on native - the port discipline is
"same IR, same ap-obj semantics, new representation."

## Design

Two crates:

- `ap-codegen-wasm`: ap-ir -> WasmGC module. Python objects become WasmGC structs:
  a `pyobj` hierarchy (int/float/str/bytes/bool/none instances, container structs,
  function closures, type objects with method tables). Big-int arithmetic lowers to
  an imported runtime or struct-based bignum routines. Exceptions: Wasm EH
  (exnref) with a Python-exception struct carrying type/traceback refs. Generators:
  explicit state structs. The representation design is done ONCE here with p2w
  (MIT) as the design reference - p2w proved the mapping on a subset; we cite its
  document, we do not port its Python code.
- `ap-host`: wasmtime-based runner with WasmGC enabled, WASI imports (stdio, clock,
  args, env-lite), the runtime helper imports (bignum, format, repr), and an
  embedding API (Rust lib + later a C ABI for hosts).

Semantics invariance rule: the conformance corpus must produce byte-identical
stdout/stderr/exit-code under native AND wasm modes. Divergences get fixed in the
shared layers (ap-obj semantics, ap-ir lowering), never papered over in the wasm
codegen - that is the whole point of running one IR into two backends.

Threading: wasi-threads where the host provides them; single-threaded fallback with
a startup diagnostic when not. Documented target-dependent guarantee (master
contract), not a hidden difference.

## Leaves

### 03.01 - WasmGC representation spec
`docs/spec/07-wasm-repr.md`: the struct hierarchy, vtable layout, string repr
(must round-trip CPython str exactly incl. all three internal encodings), bignum
strategy, exception object shape, closure layout. Every choice cites either a spec
clause or a p2w-documented precedent. Gate: main-model review; the doc is complete
enough that codegen leaves never make a representation decision unreviewed.

### 03.02 - ap-codegen-wasm values + control flow
Lower values, arithmetic (incl. bigint strategy), control flow, function calls,
closures to a wasm module that runs under ap-host. Harness flag:
`ap-conform run --impl 'ap-host run'`. Gate: values corpus slice byte-identical
native-vs-wasm.

### 03.03 - ap-codegen-wasm classes + exceptions
Type objects as wasm structs with method tables, MRO precomputed at build, attribute
lookup lowered per spec, Wasm EH mapping for try/except/finally with CPython-format
traceback reconstruction at the host boundary. Gate: classes + exceptions slices
byte-identical.

### 03.04 - ap-codegen-wasm generators, match, comprehensions
Generator state structs (the p2w-cited design), match lowering, comprehension
inlining. Gate: full corpus byte-identical native-vs-wasm under tier-0-equivalent
(wasm has no tier-1 in v1 - document as known gap, cite).

### 03.05 - ap-host WASI + runtime imports
stdio/stderr, exit codes, clock, env/args (capability-scoped), the bignum/format
import surface finalized and versioned. Gate: ap-host runs the FULL corpus in wasm
mode green; startup time measured and logged (target: <50ms to first byte of output
for hello-world, measured not asserted).

### 03.06 - Browser smoke + threading
One HTML demo embedding ap-host compiled to wasm (wasmtime browser build or
documented alternative - decision recorded in the leaf), running a corpus script.
wasi-threads spike: corpus threading slice under ap-host with thread support on;
document result either way (this is a feasibility probe, not a gate).

## Dependencies
Plan 02 (ap-obj semantics stable), spec 07 gate before any codegen leaf.

## Dispatch notes
03.01 first (it is the contract), then 03.02 -> 03.03 -> 03.04 sequential (each
builds on the module emitter), 03.05 parallel with 03.03 onward, 03.06 last.
