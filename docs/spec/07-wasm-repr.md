# Spec 07 — WasmGC Representation

Clause prefix: `S-07-*`. This spec is normative for Plan 03 (the Wasm backend).
It formalizes `docs/03-wasm-representation-design.md` (the design draft) into
the spec-pack clause style; where a clause and the design draft disagree, this
spec wins and the draft gets a status-line correction. Codegen leaves (03.02
onward) MUST NOT make a representation decision that is not written here.

Normative keywords are **MUST**, **MUST NOT**, **SHOULD**, and **MAY**
(RFC-2119). Every design choice cites its source: a spec clause
(`S-01-*`/`S-02-*`/`S-03-*`), a compat-contract clause (`C-*`), a p2w-documented
precedent ("p2w-precedent", abilian/p2w, MIT — design reference only, code
never ported), or a section of the design draft ("design §n"). "ap-obj" refers
to `crates/ap-obj` (the native value model, spec 01); "ap-run" to the native
executor.

Reference behavior: CPython 3.14 via the compatibility contract
(`docs/compat-contract.md`, C-01). Port discipline (Plan 03 orchestrator):
**same IR, same ap-obj semantics, new representation.** Any observable
divergence native-vs-wasm found by the differential corpus (leaf 03.07) is
fixed in the shared layers (ap-obj / ap-ir), never papered over in wasm
codegen.

Host/engine constraint (design §0, measured): emitted modules use WasmGC
struct/array types and Wasm exception handling (`throw`/`try_table`/`exnref`),
so they require a Wasm-3.0-class engine. **ap-host pins `wasmtime = "47"`**
(WasmGC + EH on by default); **wasmtime ≥ 37 with
`Config::wasm_exceptions(true)` is the floor** for exception-carrying modules.
This constraint is normative (see S-07-40).

---

## 1. Value representation — uniformly boxed `(ref null $pyobj)`

- **S-07-1 (MUST)** Every Python value on the wasm operand stack, in locals,
  and in struct fields is a `(ref null $pyobj)` — a WasmGC struct reference
  whose head type is `$pyobj`. No NaN-boxing, no `i31ref` value tagging in v1.
  Source: ap-obj's native `Value` is a 16-byte enum with inline smi/float;
  representation choice is explicitly backend-local under the port discipline.
  WasmGC's `i31ref` holds 31 bits; the ap-obj smi contract is **i61**
  (`SMI_BITS = 60`, `crates/ap-obj/src/int.rs`), which `i31ref` cannot carry
  without introducing a second integer path — one boxed path keeps ONE code
  path per operation (C-12/C-13 exactness is preserved trivially). Cost: one
  allocation per literal, accepted. (p2w-precedent: p2w boxes all values as
  GC structs with a type tag; its native-type-eligibility inference is an
  optimization phase deferred here.)
- **S-07-2 (MUST)** `None`, `True`, `False`, and `Ellipsis` are module-level
  singleton globals allocated once in `rt_init` (S-07-30); `is` lowers to
  `ref.eq`. Reference equality survives a moving collector — Python identity
  is object-reference identity, never address identity (S-01-30/31/32).
  Corpus anchors stay the spec-01 ones: `x is None` is THE identity test;
  `True is 1` is False while `True == 1` is True (S-01-32) — hence True/False
  are distinct singletons from the ints 1/0 even though both are `$int`
  structs (S-07-8).
- **S-07-3 (MUST)** Wasm `null` is an internal poison value only: it may exist
  in a local between definition and use, but the lowering MUST NOT let it
  reach any Python-visible position. A read of a not-yet-bound name lowers to
  the ap-obj UnboundLocalError path (S-02-2), never to `null` propagation.
- **S-07-4 (MUST)** The Python-level dynamic type — what `type(x).__name__`
  prints, what user classes produce — comes exclusively from field 0 of
  `$pyobj` (the `$type` reference), never from the wasm struct type. Wasm
  struct subtyping (`ref.test`/`ref.cast`) is an internal dispatch mechanism
  only; observable behavior MUST NOT depend on it (cf. S-01-31: internal
  representation is not observable).

## 2. The `$pyobj` struct hierarchy

All structs declare `sub $pyobj`; field 0 is `$type (ref null $type)`. The
wat shapes below are normative *layouts*; the emitter MAY choose equivalent
encodings of the same fields (e.g. field ordering that keeps ref fields
together) as long as every clause here holds.

### 2.1 Integers

```wat
(type $pyobj (sub (struct (field $type (ref null $type)))))
(type $int (sub $pyobj (struct
  (field $type (ref null $type))
  (field $kind i32)                    ;; 0 = smi, 1 = big
  (field $smi  i64)                    ;; valid iff kind=0; i61 contract
  (field $big  (ref null $bigint)))))  ;; valid iff kind=1
(type $bigint (struct                  ;; internal payload, NOT a $pyobj
  (field $sign i32)                    ;; -1 / +1
  (field $limbs (ref $limb_array))))   ;; little-endian u32 limbs
(type $limb_array (array (mut i32)))
```

- **S-07-5 (MUST)** `$int` carries a kind tag: `kind=0` means the `i64 $smi`
  field holds the value under the i61 contract (value in `[-2^60, 2^60)`,
  `SMI_BITS = 60` — ap-obj `int.rs`, C-13: no wrapped result may escape);
  `kind=1` means the value lives in `$big`. Arithmetic on two smis compiles
  to `i64` ops with an overflow check promoting to the bigint routine at the
  i61 escape boundary — exactly ap-obj's `Int::Smi` promotion boundary
  (C-13; corpus sweeps `2^60 ± k`). The boundary is invisible (S-01-1,
  C-12): `int` is arbitrary precision, no operation overflows or loses
  precision.
- **S-07-6 (MUST)** `$bigint` is an internal payload struct, not a `$pyobj`:
  it is reachable only through `$int.$big`, so it cannot appear at a
  Python-visible position (consistent with S-07-3). Limbs are little-endian
  `u32` — the same limb layout ap-obj's `BigInt` (num-bigint) backend uses,
  so the shared-source byte-exactness argument of S-07-19 applies unchanged.

### 2.2 Float, bool, none/ellipsis

```wat
(type $float (sub $pyobj (struct
  (field $type (ref null $type))
  (field $val f64))))                  ;; IEEE-754 binary64
```

- **S-07-7 (MUST)** `$float` stores an `f64`; arithmetic is IEEE-754 binary64
  round-to-nearest-even (S-01-8, C-15/C-16). `repr`/`str` byte-exactness
  (S-01-41, C-17/C-18) comes from the shared shortest-round-trip algorithm
  ported into the runtime prelude (S-07-34), never re-derived in the host.
- **S-07-8 (MUST)** There is NO dedicated bool struct. `True`/`False` are
  `$int` singletons with `kind=0`, `smi=1/0`, whose field 0 points at the
  `bool` type object. `bool ⊂ int` (S-01-9) then falls out: arithmetic needs
  no conversion arm, and only `type()`/`repr` read `$type` (S-01-10:
  `repr(True)` == `True`, never `1`; bool arithmetic results are `int`, never
  `bool`). (p2w-precedent: p2w likewise models bool via int with a type tag.)
- **S-07-9 (MUST)** `None` and `Ellipsis` are `$pyobj` instances (bare, no
  payload beyond field 0) with fixed type objects, one global each
  (S-07-2, S-01-32).

### 2.3 Strings and bytes

```wat
(type $str (sub $pyobj (struct
  (field $type (ref null $type))
  (field $hash i64)                    ;; cached; 0 = unhashed (S-01-38)
  (field $kind i32)                    ;; 0 = ascii/latin1, 1 = ucs2, 2 = ucs4
  (field $len  i32)                    ;; code points (S-01-14)
  (field $data (ref null $byte_arr))))) ;; kind=0 payload: len bytes
(type $str16 (sub $str (struct ... (field $data16 (ref $i16_arr)))))
(type $str32 (sub $str (struct ... (field $data32 (ref $i32_arr)))))
(type $byte_arr (array i8))
(type $i16_arr (array i16)) (type $i32_arr (array i32))
(type $bytes (sub $pyobj (struct
  (field $type (ref null $type))
  (field $hash i64)
  (field $len  i32)                    ;; BYTES, not code points (S-01-18)
  (field $data (ref $byte_arr)))))
```

- **S-07-10 (MUST)** `$str` mirrors CPython's three internal encodings via
  the kind tag and the `$str16`/`$str32` subtypes: kind 0 = ascii/latin1
  (1 byte/code point, `$data`), kind 1 = UCS2 (2 bytes/cp, `$data16`), kind 2
  = UCS4 (4 bytes/cp, `$data32`). `$len` counts code points (S-01-14:
  `len("café")` == 4, `len("a\u0301")` == 2). All three encodings MUST
  round-trip byte-exactly — see S-07-35.
- **S-07-11 (MUST)** v1 emission policy: every string literal and string
  result whose code points all fit latin1 uses kind 0. A string produced with
  any code point > 0xFF starts as UTF-8-in-`$data` with a cached code-point
  count — exactly ap-obj's `str_repr.rs` storage split (CompactAscii vs
  Unicode) — and `$str16`/`$str32` are declared now but only emitted when the
  O(n) UTF-8 indexing cost shows up in a corpus profile. Rationale: the
  subtypes existing up front makes widening a codegen-local change, not a
  representation break (design §2). Both policies are invisible per
  S-01-31/S-07-4: observable behavior MUST NOT depend on which internal
  encoding a given string uses.
- **S-07-12 (MUST)** Cached string hash: `$hash` == 0 means unhashed; once
  computed it holds the siphash13-family value per S-01-38 — salted per
  process, never asserted byte-wise by the corpus, but equal strings hash
  equal in-process and dict/set behavior keyed on strings is internally
  consistent. Same rule for `$bytes.$hash` (S-01-38). `hash(int)` values
  themselves (S-01-36) are computed per S-07-19, not cached here.
- **S-07-13 (MUST)** `$bytes` stores raw bytes in a `$byte_arr`; `len` counts
  bytes (S-01-18); bytes iter/index yield `$int` (S-01-18: `b"Hi"[0]` == 72).
  str/bytes never implicitly convert (S-01-17) — the wasm lowering of every
  operator checks `$type`, same as ap-obj.

### 2.4 Containers

```wat
(type $obj_arr    (array (mut (ref null $pyobj))))
(type $obj_arr_ro (array (ref null $pyobj)))       ;; tuple storage, immutable
(type $list (sub $pyobj (struct
  (field $type  (ref null $type))
  (field $len   i32) (field $cap i32)
  (field $items (ref $obj_arr)))))
(type $tuple (sub $pyobj (struct                    ;; built once via array.new_fixed
  (field $type  (ref null $type))
  (field $items (ref $obj_arr_ro)))))
(type $dict_entry (struct
  (field $hash i64) (field $key (ref null $pyobj))
  (field $value (ref null $pyobj))))
(type $entry_arr (array (mut $dict_entry)))
(type $idx_arr (array (mut i32)))
(type $dict (sub $pyobj (struct                      ;; CPython compact layout
  (field $type (ref null $type))
  (field $used i32) (field $fill i32)
  (field $mask i32) (field $perturb_shift i32)
  (field $index   (ref $idx_arr))     ;; DKIX_EMPTY / DKIX_DUMMY / entry index
  (field $entries (ref $entry_arr)))))
(type $set (sub $pyobj (struct                       ;; same probing, no values
  (field $type (ref null $type))
  (field $used i32) (field $fill i32)
  (field $mask i32) (field $perturb_shift i32)
  (field $index   (ref $idx_arr))
  (field $entries (ref $entry_arr))))) ; $entry_arr with $value = null entry-structs
```

- **S-07-14 (MUST)** `$list`: length/capacity/items array; mutation in place,
  aliasing on assignment (S-01-19). `$tuple`: immutable — built once via
  `array.new_fixed` into an immutable array type; item deletion raises the
  S-01-19 TypeError (type check before any storage access).
- **S-07-15 (MUST)** `$dict` is a direct port of the CPython compact layout
  as already implemented in ap-obj (spec 01 cites): sparse index array +
  dense entries array (S-01-21), 2/3 load factor, perturb probing
  `i*5 + perturb + 1`, growth `used*3` → next power of two. Insertion order
  (S-01-21), order-independent equality (S-01-22), hash+equality key
  collapsing `1/1.0/True` (S-01-23), and exact unhashable-key/missing-key
  error text (S-01-24) are ALGORITHM properties fixed by the native
  implementation; the wasm backend ports the algorithm, not a new design.
  Byte-identical iteration order is gated by the differential corpus.
- **S-07-16 (MUST)** `$set` uses the same probing machinery without values;
  element order in iteration/repr stays UNSPECIFIED (S-01-25) and the corpus
  sorts multi-element sets before comparing. `frozenset` is a distinct
  `$type` over the same `$set` layout (immutable usage), carrying the
  S-01-26 semantics: hashable, order-independent equality, equal frozensets
  hash equal, repr `frozenset({1, 2})`. Set/frozenset repr ORDERING rules
  follow C-26 (sorted-compare discipline is a test rule, not a storage rule).

### 2.5 Instances

- **S-07-17 (MUST)** A user-class instance is ONE generic struct:
  ```wat
  (type $instance (sub $pyobj (struct
    (field $type  (ref null $type))    ;; the dynamic class
    (field $attrs (ref null $dict))))) ;; lazy: null until first attribute set
  ```
  Attribute lookup follows the ap-obj order: data descriptors → instance
  `$attrs` dict → non-data descriptors (S-03-09/10/12); `$attrs` staying
  null for a fresh instance is invisible (S-01-31). Per-class `__slots__`
  field layouts are a documented non-goal for v1 (S-07-48) — `__slots__`
  classes still work through the generic-attrs path, preserving observable
  behavior (S-03-13) at v1 performance cost.

## 3. Type objects and the slot-id method table

```wat
(type $type_arr (array (mut (ref null $type))))
(type $slot_arr (array (mut (ref null $function))))
(type $type (sub $pyobj (struct
  (field $type  (ref null $type))      ;; metatype (usually `type`), S-03-2
  (field $name  (ref $str))
  (field $mro   (ref $type_arr))       ;; C3 linearization, PRECOMPUTED (S-03-15)
  (field $slots (ref $slot_arr))       ;; slot-id indexed method table
  (field $bases (ref $obj_arr))
  (field $dict  (ref $dict)))))        ;; class __dict__
```

- **S-07-18 (MUST)** `$mro` is the C3 linearization computed at class-build
  time (S-03-15) and stored as an array; `isinstance`/`issubclass` walk it
  (S-03-31); `super()` uses its index arithmetic (S-03-17/18). The MRO is
  never recomputed at call time.
- **S-07-19 (MUST, slot-id table)** `$slots` is indexed by **slot id**: a
  dense `u32` enum of the special methods the runtime dispatches implicitly,
  enumerated ONCE here and shared verbatim with the native backend's dispatch
  enum (a new slot id is appended in BOTH backends in the same commit; ids
  are never renumbered). Slot resolution walks `$mro` at class-build time and
  fills inherited entries eagerly, so runtime implicit special-method dispatch
  (S-03-22, bypass on instances — S-03-23) is exactly
  `struct.get $slots[id]`, with the absent-entry sentinel triggering the
  S-03-24 fallback. Reflected-operand order (S-03-25) and augmented
  assignment (S-03-26) are lowering rules over the same table. The full
  slot-id list (v1 order, alphabetical within groups):

  | ids | slots |
  |-----|-------|
  | 0–7 | `__bool__` `__len__` `__iter__` `__next__` `__getitem__` `__setitem__` `__delitem__` `__contains__` |
  | 8–13 | `__call__` `__repr__` `__str__` `__hash__` `__eq__` `__ne__` |
  | 14–21 | `__lt__` `__le__` `__gt__` `__ge__` `__neg__` `__pos__` `__abs__` `__invert__` |
  | 22–37 | `__add__` `__radd__` `__sub__` `__rsub__` `__mul__` `__rmul__` `__truediv__` `__rtruediv__` `__floordiv__` `__rfloordiv__` `__mod__` `__rmod__` `__pow__` `__rpow__` `__lshift__` `__rlshift__` |
  | 38–47 | `__rshift__` `__rrshift__` `__and__` `__rand__` `__xor__` `__rxor__` `__or__` `__ror__` `__int__` `__index__` |
  | 48–59 | `__iadd__` `__isub__` `__imul__` `__itruediv__` `__ifloordiv__` `__imod__` `__ipow__` `__iand__` `__ior__` `__ixor__` `__ilshift__` `__irshift__` |
  | 60–67 | `__enter__` `__exit__` `__get__` `__set__` `__delete__` `__setattr__` `__getattr__` `__getattribute__` |
  | 68–75 | `__delattr__` `__del__` `__await__` `__aiter__` `__anext__` `__format__` `__fspath__` `__length_hint__` |

  Names NOT in this table (`__new__`, `__init__`, `__init_subclass__`,
  `__mro_entries__`, `__set_name__`, `__subclasshook__`, `__instancecheck__`,
  dunder attributes like `__class__`/`__name__`/`__dict__`/`__code__`, ...)
  live in the class `$dict` and resolve through the attribute path
  (S-03-05/06/07, S-03-12), because they are looked up by NAME (explicitly or
  by the creation/instantiation pipelines), not dispatched by position.
  (p2w-precedent: p2w emits per-class dispatch tables with eagerly resolved
  entries; the slot-id enumeration itself is ours, keyed off ap-obj.)

## 4. Callables and closures

```wat
(type $cell_arr (array (ref $cell)))
(type $code (struct                       ;; one per FnDef; module data, immutable
  (field $name        (ref $str))         ;; frame name, S-02-38
  (field $nparams     i32)
  (field $flags       i32)                ;; posonly/vararg/kwonly/kwarg bits
  (field $param_names (ref $obj_arr))
  (field $func_idx    i32)))              ;; wasm function-table index (§5)
(type $function (sub $pyobj (struct
  (field $type       (ref null $type))
  (field $code       (ref $code))
  (field $defaults   (ref $obj_arr))     ;; evaluated at def time, S-02-7
  (field $kwdefaults (ref null $dict))
  (field $cells      (ref $cell_arr))    ;; closure environment
  (field $globals    (ref $dict)))))     ;; module namespace (LEGB, S-02-1/3)
(type $cell (struct (field $v (mut (ref null $pyobj)))))
(type $method (sub $pyobj (struct         ;; bound method
  (field $type (ref null $type))
  (field $self (ref $pyobj))
  (field $fn   (ref $function)))))
```

- **S-07-20 (MUST)** A cell is the shared mutable box: an inner function
  referencing an enclosing variable captures the CELL, not the value
  (S-02-12); all closures over the same variable share ONE cell (S-02-13);
  closure lifetime is independent of the defining frame (S-02-14 — under
  engine GC this is automatic: the `$function` keeps its `$cells` reachable).
  Late binding falls out of the shared cell (S-02-12).
- **S-07-21 (MUST)** `$function.$defaults` is the def-time-evaluated default
  list (S-02-7 evaluation order); `$kwdefaults` the keyword-only defaults;
  `$code.$flags` encodes the parameter-kind bitmap of S-02-8 and binding
  errors are raised with ap-run's exact text (S-02-10) by the prelude binder.
- **S-07-22 (MUST)** Bound methods are `$method` structs pairing self + a
  `$function`; attribute access on an instance whose class dict holds a
  function produces one (descriptor protocol via the `__get__` slot id,
  S-03-20). Function attributes (`__name__`, `__code__`, `__defaults__`,
  `__globals__`, `__qualname__`) read from the corresponding `$code`/
  `$function` fields so introspection matches the reference (spec 05 owns
  the observable surface).

## 5. Calling convention

- **S-07-23 (MUST)** All Python-level wasm functions share ONE signature:
  ```wat
  (func $pycall (param $fn (ref null $function))
                (param $args (ref $obj_arr))
                (param $kwargs (ref null $dict))
                (result (ref null $pyobj)))
  ```
  The function body binds parameters per S-02-8..S-02-10 (ap-run's binding
  algorithm and error text, ported) before the lowered body. `return` yields
  the operand or the `None` singleton (S-02-11).
- **S-07-24 (MUST)** Static specialization rule (design §3): a call the
  lowering PROVES static — direct call to a module-level `def` with no
  decorator/metaclass interference — compiles to a direct `call $f_N` with
  positional locals, skipping the generic packing. The uniform convention of
  S-07-23 is the FALLBACK (dynamic calls, bound methods, callables held in
  values), not the fast path. Specialization is invisible (S-01-31, C-31):
  observable behavior MUST NOT differ between the two paths — the
  differential corpus gates this.
- **S-07-25 (MUST)** There is NO error-value convention: raising is
  `throw $tag_pyexc` (S-07-28) on every failure path. The p2w-style
  `(result value, is_error)` pair is explicitly not used (p2w targets
  browsers where EH availability was uneven; wasmtime ≥ 47 has EH by
  default, design §3). A wasm function that can raise either returns normally
  or throws — never a sentinel value.

## 6. Bigint strategy — DECISION: struct-based, in-module

- **S-07-26 (MUST)** Arbitrary-precision integer arithmetic is implemented
  IN-MODULE: `$bigint` structs (S-07-6) with arithmetic routines compiled
  into the module (the runtime prelude, S-07-33). Host-imported bignum
  functions MUST NOT exist. Recorded reasoning (design §4):
  1. **Artifact portability** — master.md ships two artifacts (native binary
     + separate .wasm); leaf 03.06 runs the module in a browser. Host-imported
     bignum makes the .wasm non-self-contained and ties arithmetic semantics
     to a second implementation that must be kept in lockstep for
     byte-exactness.
  2. **Boundary cost + GC rooting** — every bigint op across the host
     boundary pays a host call plus rooting ceremony for GC-ref arguments;
     struct-based ops stay inside the engine's collector.
  3. **Shared-source byte-exactness** — the limb arithmetic, `hash(int)`
     modulus mapping (S-01-36), and `int.__repr__` digit algorithm are
     transcriptions of CPython already corpus-proven in ap-obj; they compile
     to wasm32 as pure Rust (num-bigint is no_std-friendly), and the prelude
     links the SAME crate versions the native backend uses, so "same
     semantics" is enforced by shared source rather than a parallel port
     (C-12/C-13/C-14).
  4. **p2w-precedent** — p2w implements bignum entirely in-module (its
     runtime library phase) and its mapping document shows the approach
     passing its 104-program suite.
- **S-07-27 (MUST)** Consequence for ap-host: the runtime helper import
  surface NARROWS — bignum, `format`, and `repr` are prelude functions too
  (they build `$str` GC structs, which is awkward across the host boundary
  for zero benefit). ap-host's imports are WASI p1 (stdio/clock/args/env) +
  the `aprt` service surface (S-07-41). This narrows the Plan 03 orchestrator
  sketch ("runtime helper imports (bignum, format, repr)") and is recorded in
  the leaf 03.05 brief; it does not re-litigate a master.md locked decision
  (two artifacts and the crate split are both preserved).

## 7. Exceptions — Wasm EH, wasmtime ≥ 47 (≥ 37 behind a flag)

```wat
(tag $tag_pyexc (param (ref $exception)))
(type $tb_frame (struct
  (field $next     (ref null $tb_frame))   ;; toward outer frames
  (field $filename (ref $str)) (field $funcname (ref $str))
  (field $lineno i32) (field $col i32) (field $end_lineno i32)))
(type $exception (sub $pyobj (struct
  (field $type   (ref null $type))          ;; the exception CLASS
  (field $args   (ref $tuple))
  (field $tb     (ref null $tb_frame))      ;; traceback head
  (field $context (ref null $exception))    ;; implicit chaining, S-02-34
  (field $cause  (ref null $exception))     ;; raise ... from ..., S-02-28
  (field $suppress_context i32))))
```

- **S-07-28 (MUST)** `raise EXPR` builds a `$exception` (args tuple and chain
  fields per S-02-27/28/34), fills `$tb` from the frame's line marker
  (`Stmt::Line`), then `throw $tag_pyexc`. `try/except` lowers to
  `try_table` with `(catch $tag_pyexc $L)`; the handler receives the
  `(ref $exception)` and runs ap-obj's except-match algorithm — clause order
  in source (S-02-30), single class or tuple of classes via `issubclass` over
  the precomputed MRO (S-03-31), bare `except:` catches any BaseException
  (S-02-31). `try/else` per S-02-32.
- **S-07-29 (MUST)** `finally` lowers to the `try_table (catch_all_ref ...)`
  rethrow pattern; the finalizer runs on EVERY exit path including EH ones
  (S-02-33). `break`/`continue` through `finally` are lowered as ap-run
  lowers them (pending-jump values), NOT via exception handling.
  `with` uses the `__enter__`/`__exit__` slot ids (S-02-40/41/42).
- **S-07-30 (MUST)** exnref payload rule: the Python exception travels as
  `(ref $exception)` in the tag payload — wasmtime builds exception objects
  on the GC heap, so GC refs in payloads are sound. An i32 index into a side
  table MUST NOT be used (it would break under the moving collector and
  lose the object identity of `e`). Uncaught exceptions escaping `_start`
  are rendered at the ap-host boundary in CPython traceback format
  (S-02-35/36/37/38, C-25/C-29): the frames come from the `$tb` chain the
  lowered code maintained; source lines come from a per-module source-map
  data section; exit code 1 (C-29/C-30).
- **S-07-31 (MUST)** Version constraint (normative): exception-carrying
  modules require **wasmtime ≥ 37 with `Config::wasm_exceptions(true)`, or
  ≥ 47 where WasmGC and EH are default-on; ap-host pins `wasmtime = "47"`**.
  A pre-EH error-value fallback is explicitly NOT implemented — two calling
  conventions would double the codegen surface for zero corpus gain
  (design §5).

## 8. Generators

```wat
(type $generator (sub $pyobj (struct
  (field $type   (ref null $type))
  (field $fn     (ref $function))     ;; code + cells to resume with
  (field $state  i32)                 ;; 0=suspended-at-entry, N=yield site N,
                                      ;; -1=exhausted, -2=closed
  (field $locals (ref $obj_arr))      ;; spilled frame slots
  (field $value  (ref null $pyobj))   ;; last sent / to-be-yielded value
  (field $exn    (ref null $exception))))) ;; pending throw-in (gen.throw)
```

- **S-07-32 (MUST)** A function body containing `yield` compiles to a
  generator (S-02-20): ONE wasm function with a `block`/`br_table` dispatch
  on `$generator.$state` — the classic state-machine switch (p2w-precedent:
  p2w lowers `yield` to numbered suspension points resumed by a dispatch
  table). Each `yield` stores live locals into `$locals`, sets `$state`,
  and returns the yielded value. `send`/`next`/`throw`/`close` semantics are
  ap-run's: first-send-must-be-None (S-02-24), StopIteration plumbing
  (S-02-22) and PEP 479 inside the body (S-02-23), close → GeneratorExit
  throw-in (S-02-25), `yield from` delegation (S-02-26). No stack switching,
  no asyncify: this is the same algorithm the native executor's generator
  frames implement, with a struct instead of a heap frame. Wasm
  stack-switching (`cont`/`resume`, experimental in wasmtime 47, x86_64
  Linux only) is a future performance path, not a v1 dependency (S-07-48).

## 9. Strings: internal encodings and round-trip

- **S-07-33 (MUST)** Runtime prelude: the byte-exact ALGORITHMS — float
  shortest-round-trip repr (`float_repr.rs`, S-01-41/C-15), str repr escaping
  and quote selection (`str_repr.rs`, S-01-42), bigint decimal formatting and
  parsing (S-01-40, C-14), int/float hashing (S-01-36/37/38), dict/set
  machinery (S-07-15/16), and the parameter binder (S-02-8..10) — are ported
  as Rust compiled into each emitted module ("prelude Rust"), linking the
  same crate versions the native backend uses. The host never re-derives a
  Python value's text; print/repr/format build `$str` structs in-module and
  hand bytes to `fd_write`. Same-source is the enforcement mechanism for
  byte-exactness in both backends.
- **S-07-34 (MUST)** String round-trip contract (orchestrator 03.01): every
  string, in all three internal encodings — ascii/latin1 (kind 0), UCS2
  (kind 1), UCS4 (kind 2) — MUST survive the round trip
  `str source literal → internal $str → repr()/str() output` BYTE-EXACT:
  - ascii/latin1 strings: repr/str bytes identical to the reference,
    including quote selection and `\xNN` escapes (S-01-42);
  - non-latin1 (BMP, ucs2-representable): printable non-ASCII passes through
    unescaped in repr (S-01-42: `repr("café 中")` → `'café 中'`), encode/
    decode to UTF-8 round-trips (S-01-18);
  - astral (ucs4, e.g. `\U0001F600`): repr passes the character through
    unescaped (S-01-42), `len` counts it as one code point (S-01-14), and
    indexing/slicing treat it as one unit (S-01-15).
  Under the v1 emission policy (S-07-11) astral strings live in
  UTF-8-in-`$data` until the widening lands; the round-trip contract applies
  to the VALUES regardless of which internal encoding holds them — a corpus
  case printing any of the three MUST byte-match the reference in wasm mode.
  Verification: leaf 03.02's values corpus slice includes latin1, BMP, and
  astral repr cases; leaf 03.07 gates the full corpus.
- **S-07-35 (MUST)** `str` ↔ `bytes` conversions (`encode`/`decode`, defaults
  UTF-8) and their exact error texts (S-01-18) are prelude implementations
  over `$str`/`$bytes`; no string data crosses the host boundary in v1
  except as final output bytes (S-07-41).

## 10. GC ownership — who owns object lifetime

- **S-07-36 (MUST, decision)** Inside a wasm instance, the ENGINE's WasmGC
  collector owns every Python object's lifetime. MMTk (ap-gc) never runs in
  wasm mode: the wasm backend has no `Heap` implementation — allocation is
  `struct.new`/`array.new`, collection is the engine's (wasmtime 47: Cheney
  semi-space copying; V8 in the browser: generational). The native `Heap`
  trait (ap-gc → MMTk, `docs/gc-mmtk-swap.md`) is a native-backend detail.
  What must still match is the ap-obj SEMANTICS GC exposes (C-19):
  collection timing is unobservable except through `__del__`/weakref, and
  reference cycles are collected by both engines (both trace).
- **S-07-37 (MUST, Class-X divergence)** `__del__` is NEVER called in wasm
  mode: WasmGC has no finalization API in wasmtime 47. Native already runs
  finalizers at collect time with unguaranteed timing (C-20/C-21,
  documented divergence D1); wasm-mode non-invocation is recorded as a
  **Class-X divergence** (compat-contract §9 classes, D6 regime) with a
  tracked issue. Tests MUST NOT rely on finalization timing or occurrence on
  the wasm path (C-23). Revisit trigger: wasmtime landing the
  finalization GC extension (S-07-48).
- **S-07-38 (MUST, Class-X divergence)** `weakref` is REFUSED at compile
  time in v1 wasm mode (same treatment class as C-extension imports, C-07 —
  but a distinct diagnostic token, `APX-WASM-WEAKREF`, machine-stable per
  C-10). Rationale (design §8): with a moving collector and no engine
  weak-reference support, an in-module weak table cannot observe target
  death, and host-side rooted tables keep targets alive, breaking weak
  semantics. Native weakref behavior (C-22) is unaffected. Revisit trigger:
  wasmtime landing the weak-references/finalization GC extensions.
- **S-07-39 (MUST)** Cross-boundary rooting rules: ap-host holds GC handles
  (e.g. an `OwnRooted<StructRef>` for a cached entry point, or a caught
  `(ref $exception)`) only within the wasmtime `Store` scope that produced
  them; ap-host MUST NOT retain a GC handle across a new instantiation and
  MUST NOT store wasm GC refs in native ap-obj values. Rust-side consumers
  copy payload bytes out (`$str` → bytes for stderr rendering); nothing
  aliases MMTk memory. Linear memory holds only byte buffers (I/O scratch,
  source map, data sections) — no Python object identity is ever derived
  from a linear-memory address (identity is `ref.eq`, S-07-2).

## 11. Host interface: WASI + `aprt`

- **S-07-40 (MUST)** Engine version gate: ap-host pins `wasmtime = "47"`
  and MUST refuse (clear diagnostic) to run a module when the engine cannot
  provide WasmGC + EH — i.e. any wasmtime < 37, or 37..46 without
  `wasm_exceptions(true)` explicitly enabled. Version-normative text is in
  S-07-31.
- **S-07-41 (MUST)** The module imports exactly two things: WASI preview1
  (`wasi_snapshot_preview1`: fd_write/fd_read, args, env, clock) and the
  versioned `aprt` service surface:
  ```wat
  (import "aprt" "abort_msg" (func (param (ref $str))))  ;; unrecoverable internal error
  (import "aprt" "now_ns"    (func (result i64)))        ;; time.perf_counter
  (import "aprt" "wall_ns"   (func (result i64)))        ;; time.time
  ```
  The module exports an `aprt_version` global (custom section + global);
  ap-host refuses modules from a future incompatible version with a clear
  diagnostic (leaf 03.05: "import surface finalized and versioned").
  stdout/stderr byte-exactness (C-24/C-27): print/repr/format build `$str`
  structs in-module and hand bytes to `fd_write`; the host never reformats
  Python values. Exit codes follow C-28/C-29/C-30 at the host boundary.
- **S-07-42 (MUST)** Threading (design §9): wasmtime 47 removed
  wasi-threads (#13558). v1 wasm mode is SINGLE-THREADED; when a module
  requests threads (`threading` import), ap-host emits a startup diagnostic
  and fails loudly — a documented target-dependent guarantee in the master
  contract sense, never a hidden difference. Leaf 03.06's spike evaluates
  shared-everything-threads as the replacement; the result is documented
  either way (feasibility probe, not a gate).

## 12. Module skeleton (emission order)

- **S-07-43 (MUST)** `ap-codegen-wasm` emits, in order:
  1. type section: the `$pyobj` hierarchy (§2–§8) + function types;
  2. import section: WASI p1 + `aprt` (S-07-41);
  3. function/table sections: one wasm func per `$code` (uniform signature
     S-07-23, specialized per S-07-24), `$pycall` table for `call_indirect`;
  4. globals: singletons (`None`/`True`/`False`/`Ellipsis`), rt-init flag;
  5. memory (1 page min) + data sections: string/bytes literals (raw bytes
     copied into `$byte_arr`s at init), source map for tracebacks;
  6. `(func $rt_init)`: allocate singletons + builtin type objects (with
     precomputed MROs and slot tables, S-07-18/19), interned strings;
  7. `(func $_start)` export: `$rt_init` (guarded), then module body items in
     source order; uncaught `$exception` at this boundary → ap-host renders
     the CPython-format traceback to stderr and sets exit code 1 (S-07-30);
  8. custom section `aprt_version`.
  The hello-world bootstrap in `crates/ap-codegen-wasm/src/lower.rs` +
  `crates/ap-host` (linear-memory print via `fd_write` + wasmtime runner +
  integration test) is scaffolding predating this spec; leaf 03.02 replaces
  its print path with the GC `$str` path above (cited replacement:
  S-07-10/S-07-33/S-07-43) — its ap-ir → bytes → wasmtime → captured-stdout
  pipeline proof remains the entry-gate artifact of this leaf.
- **S-07-44 (SHOULD)** String literal interning: literals MAY be laid out as
  data-section bytes and materialized into `$str` structs lazily on first
  use, or eagerly in `$rt_init` — choice is per-module (affects module size
  vs startup; leaf 03.05 measures the <50ms-to-first-byte target, measured
  not asserted). Observable behavior MUST NOT depend on the choice
  (S-01-31/S-07-4).

## 13. Open items resolved from design §11

- Slot-id enum: S-07-19 (frozen; shared with native dispatch, append-only).
- String interning policy: S-07-44 (free choice, measured in 03.05).
- `float.__repr__` shortest-round-trip: prelude Rust, same-source rule
  (S-07-33) — never emitted wasm.
- f-string `FORMAT_VALUE` surface: owned by `docs/fstring-differential.md`
  and spec 05; this spec only fixes that f-string lowering produces `$str`
  structs through the same prelude formatting functions (S-01-16).
- Set/frozenset repr ordering: C-26/S-01-25 algorithm port, not a storage
  decision (S-07-16).

## 14. Gap list (deferred items + revisit triggers)

Each gap names its revisit trigger. None of these may be silently narrowed;
a leaf touching one MUST update this list in the same commit.

| # | Gap | Why deferred | Revisit trigger |
|---|-----|--------------|-----------------|
| G-1 | `weakref` module in wasm mode (S-07-38) | no engine weak-ref support over a moving collector | wasmtime lands weak-references/finalization GC extensions |
| G-2 | `__del__` never runs in wasm mode (S-07-37) | no WasmGC finalization API in wasmtime 47 | same as G-1 (finalization extension); until then Class-X tracked issue |
| G-3 | `__slots__` per-class field layouts (S-07-17) | correctness first; generic `$attrs` is behaviorally complete | post-v1 performance pass; corpus profile showing instance-dict pressure |
| G-4 | `$str16`/`$str32` emission (S-07-11) | UTF-8-in-i8 with cached char_count is correct, indexing O(n) | corpus profile showing O(n) UTF-8 indexing cost |
| G-5 | Stack-switching generators (`cont`/`resume`, S-07-32) | experimental, platform-limited in wasmtime 47 | wasmtime stabilizes stack switching cross-platform |
| G-6 | i31ref/NaN-boxing value optimization (S-07-1) | one-code-path rule for v1 | post-v1 optimization phase; MUST NOT change observable behavior |
| G-7 | wasm-mode threading (S-07-42) | wasi-threads removed in wasmtime 47 | leaf 03.06 shared-everything-threads spike (documented either way) |
| G-8 | tier-1 execution tier in wasm mode | wasm has no tier-1 in v1 (documented known gap, leaf 03.04) | Plan 04 tier policy revisit; behavior is tier-invisible per C-31 |

## Corpus anchors

Wasm-mode parity for every spec-01/02/03 anchor is gated by leaves 03.02–03.07
(native-vs-wasm byte-diff over the existing corpus, C-27/C-33). This spec adds
no new corpus cases itself; its test gates are the T-12/T-13 test leaves
authored from it (`plans/antipythony/tests/TRACKING.md`).
