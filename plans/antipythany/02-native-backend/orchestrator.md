# Plan 02 - Native Backend

Goal: run ap-ir natively at tier-0, optimize at tier-1, and emit a single static
binary via AoT. This plan proves correctness on native before any WASM work starts.

## Design

Four crates:

- `ap-obj`: the Python object model. Monty (MIT) is the adaptation donor - its value
  representations, dispatch tables, and exception machinery are Rust, Python 3.14
  era, and small. Adapt, attribute, keep license headers. Every Python-visible
  behavior cites a spec clause.
- `ap-gc`: MMTk-core binding. Trait `Heap { alloc, gc_trigger, add_root_range }`.
  Objects move in MMTk-managed space; ap-obj holds handles, never raw pointers that
  survive safepoints. Conservative stack scanning at tier-0; precise stack maps at
  tier-1 (Cranelift user stack maps).
- `ap-run`: tier-0 tree-walking/register executor over ap-ir calling ap-obj. This is
  the correctness baseline: every conformance case must pass here before tier-1 may
  optimize it.
- `ap-codegen-native`: ap-ir -> Cranelift CLIF. Tier-1 JIT: type-feedback-driven
  specialization (inline caches on attribute load/call sites), OSR on hot loops,
  guards + deopt to tier-0 on assumption failure. AoT: cranelift-object ->
  system linker -> static binary embedding ap-obj + ap-gc + reachable stdlib.

Threading (02.07): no GIL. Per-object state either thread-confined (proven by
ownership analysis) or behind atomic/sync wrappers in ap-obj. `threading` stdlib
module maps to real OS threads.

## Leaves

### 02.01 - ap-obj core values
Boxed value repr: small-int tagging, bigint via num-bigint, float (byte-exact
repr/str per spec), str (with CPython's latin1/UCS2/UCS4 strategy), bytes, bool,
None. Equality/identity/hashing per spec clauses. Expose: `ap_obj::Value`, ops trait
surface. Test leaf: tests/05-values-from-spec.md.
Gate: ap-conform green on the values corpus slice.

### 02.02 - ap-run tier-0 executor
Execute lowered modules: frames, name lookup (fast/slow paths), control flow,
exceptions with CPython-format tracebacks, generators (stackful or explicit state -
implementer choice, cite tradeoff), context managers. Expose: `ap_run::run(module)`.
Gate: ap-conform green on FULL v1 corpus under tier-0 (`--tier 0`).

### 02.03 - ap-obj classes/protocols
Type objects, MRO (C3), attribute lookup order per spec 03, descriptors, properties,
special-method dispatch (`__init__`, `__str__`, `__repr__`, operators, context
managers, iteration protocol), metaclass hooks. Test leaf:
tests/06-classes-from-spec.md.
Gate: corpus classes/exceptions slices green under tier-0.

### 02.04 - ap-gc MMTk integration
Bind MMTk, implement Heap trait, safepoints in ap-run loops and calls, root
scanning from ap-run frames + ap-obj containers. Finalizer semantics: `__del__`
runs, timing NOT guaranteed (documented divergence - cite compat contract). Weakrefs
per spec. Gate: allocation-heavy corpus slice green; no leak across 10k iterations
(heap growth bounded).

### 02.05 - ap-codegen-native tier-1
PON LICENSE GRANTED: pre-task for this leaf is a guided study of pon's tier-1
inline-cache, OSR, and deopt implementation with a written findings note; adapt
where the license terms allow, attribute per master license table TODO.
CLIF lowering of hot functions, inline caches, guarded specialization, OSR, deopt
restore to tier-0 exact mid-state. Correctness rule: tier-1 is invisible except by
timing - any observable difference is a bug (conformance suite runs both forced
tier-0 and adaptive modes). Gate: adaptive-mode full corpus green; >=1.5x geomean
over tier-0 on the arithmetic/string benchmark slice.

### 02.06 - AoT static binary
cranelift-object pipeline, link static (musl for linux, standard for macos), embed
ap-obj+ap-gc+reachable-stdlib-only (LTO dead-strip), emit `antipythany build app.py
-o app`. Gate: `./app` runs a hello-world + a classes corpus program; binary <= 15MB
musl; conformance suite runs against the BINARY, not the JIT, for a 30-case slice.

### 02.07 - Native threading
threading module on OS threads; object-state thread-safety audit across ap-obj
(typed per-object: Send/Sync discipline documented); GIL-free guarantee documented
with the scalings test (multi-threaded CPU corpus case scales on N cores).
Gate: corpus threading slice green; scaling case >= 3x on 4 cores.

### 02.08 - v1 stdlib subset
The exact module list from spec 05, implemented natively (sys, os-lite, math,
json->pure-python port check, re, itertools, functools, collections, ...). Each
module: port from CPython's pure-Python source where possible (PSF license - add to
notices) or native Rust impl citing clauses. Expose: module registry.
Gate: ap-conform stdlib slice green in tier-0 AND adaptive AND binary modes.

## Dependencies
01 complete (IR stable enough to execute). Spec pack sections cited throughout.

## Dispatch notes
02.01 -> 02.02 -> 02.03 -> 02.04 (02.03/02.04 parallelizable after 02.02), then
02.05 -> 02.06, 02.07/02.08 parallel at the end. This is the largest plan; if a leaf
overruns 256K context, split it along its named expose/gate boundary rather than
widening the leaf.
