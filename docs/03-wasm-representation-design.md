# Plan 03 — WasmGC representation design (pyobj → wasm structs)

Status: FORMALIZED — normative spec is `docs/spec/07-wasm-repr.md` (leaf 03.01,
S-07-* clauses). Where this draft and the spec disagree, the spec wins. Scope: how the ap-obj value model
(`crates/ap-obj/src/value.rs`, spec 01) is represented inside a WasmGC module
emitted by `ap-codegen-wasm`, and what `ap-host` (wasmtime) must provide.

Port discipline (orchestrator, `plans/antipythony/03-wasm-backend/orchestrator.md`):
**same IR, same ap-obj semantics, new representation.** Any observable
divergence found in the differential corpus is fixed in the shared layers
(ap-obj / ap-ir), never papered over in wasm codegen.

Design reference: **abilian/p2w** (MIT, Python → WAT with WASM GC;
https://git.sr.ht/~sfermigier/p2w, see its `docs/architecture.md` for the
type system, runtime architecture and calling convention). Per master.md
the p2w policy is *design reference only — cite its mapping document, port
no code* (it is written in Python against its own AST, we lower ap-ir from
Rust). Where a choice below matches p2w, it is cited as
"p2w-precedent: <topic>".

## 0. Host/engine version constraint (measured, 2026-10)

- Wasmtime implemented the full Wasm exception-handling proposal in
  **37.0.0** (2025-09-20, behind `-W exceptions` / `Config::wasm_exceptions(true)`).
- Wasmtime **47.0.0** (2026-07-20) enables **both WasmGC and Wasm EH by
  default** (PRs #13594, #13603; both proposals are now merged into Wasm 3.0).
  47.0.0 also **removed wasi-threads** (#13558) — see §8.
- Constraint recorded: **ap-host pins `wasmtime = "47"`** (workspace member
  `crates/ap-host`). The emitted modules use WasmGC struct/array types and
  `try_table`/`throw`/`exnref`, so they require a Wasm-3.0 engine; anything
  older than wasmtime 37 cannot run exception-carrying modules at all.
- wasmtime 47's built-in GC is a **Cheney-style semi-space copying
  collector** (Bytecode Alliance, "GC and Exceptions in Wasmtime"): objects
  MOVE. Everything below must therefore avoid address-based identity
  (Python identity is object-reference identity, which a moving collector
  preserves — never compare linear-memory addresses of GC refs).

## 1. Value representation: uniformly boxed `(ref null $pyobj)`

Every Python value on the wasm operand stack / in locals / in struct fields
is a `(ref null $pyobj)` — a reference to a WasmGC struct whose head subtype
is `$pyobj`. No NaN-boxing, no i31ref tagging in v1:

- ap-obj's native `Value` is a 16-byte enum with inline smi/float; that
  optimization is *representation-local* to native and is exactly what the
  port discipline lets each backend choose. WasmGC offers `i31ref` (31-bit)
  but the ap-obj smi contract is **i61** (`SMI_BITS = 60`, ap-obj lib docs) —
  i31 cannot carry it without a second big path, so boxing every value costs
  one allocation per literal but keeps ONE code path per operation.
  (p2w-precedent: p2w boxes all values as GC structs with a type tag; its
  "native type eligibility" inference is an optimization phase we defer.)
- `None`, `True`, `False`, `Ellipsis` are **module-level singleton globals**
  (`(global $g_none (ref $pyobj) ...)`) initialized in the module start/rt-init
  function. Identity semantics (`is`) fall out of reference equality
  (`ref.eq`), which survives a copying collector.
- Nullability: `null` is an *internal* poison value only (uninitialized
  locals between definition and use); the lowering must never let `null`
  escape to a Python-visible position. `local.get` of a not-yet-bound name
  lowers to the ap-obj UnboundLocalError path, not to `null` propagation.

Struct subtyping (WasmGC `sub` declarations) gives the fast internal
dispatch: `ref.test $str`, `ref.cast $int`, etc. The Python-level dynamic
type (what `type(x).__name__` prints, user classes) always comes from the
`$type` field, never from the wasm struct type.

## 2. The `$pyobj` struct hierarchy

All structs `sub $pyobj`. Field 0 of `$pyobj` is the dynamic type pointer.

```wat
(type $pyobj (sub (struct (field $type (ref null $type)))))

;; ---- scalars ----------------------------------------------------------
(type $int  (sub $pyobj (struct
  (field $type (ref null $type))
  (field $kind i32)                     ;; 0 = smi(i64), 1 = big
  (field $smi  i64)                     ;; valid iff kind=0; i61 contract
  (field $big  (ref null $bigint)))))   ;; valid iff kind=1
(type $bigint (struct                   ;; NOT a $pyobj: internal payload
  (field $sign i32)                     ;; -1 / +1
  (field $limbs (ref $limb_array)))))   ;; little-endian u32 limbs
(type $limb_array (array (mut i32)))

(type $float (sub $pyobj (struct
  (field $type (ref null $type))
  (field $val f64))))                   ;; IEEE-754 binary64, C-16

;; bool: NO dedicated struct. True/False are $int singletons with kind=0,
;; smi=1/0, $type = the `bool` type object. bool ⊂ int (S-01-9): arithmetic
;; needs no conversion arm; only type()/repr read $type. (p2w-precedent:
;; p2w likewise models bool via int with a type tag.)

;; none/ellipsis: $pyobj instances with fixed type objects, one global each.

;; ---- strings / bytes ---------------------------------------------------
(type $str (sub $pyobj (struct
  (field $type  (ref null $type))
  (field $hash  i64)                    ;; cached; 0 = unhashed (siphash13, S-01-36/37)
  (field $kind  i32)                    ;; 0 = ascii/latin1 (1 byte/cp),
                                        ;; 1 = ucs2 (2 bytes/cp), 2 = ucs4 (4 bytes/cp)
  (field $len   i32)                    ;; code points
  (field $data  (ref null $byte_arr))))) ;; latin1: i8 array, len bytes
(type $str16  (sub $str (struct ... (field $data16 (ref $i16_arr)))))
(type $str32  (sub $str (struct ... (field $data32 (ref $i32_arr)))))
(type $byte_arr (array i8))  (type $i16_arr (array i16))  (type $i32_arr (array i32))

(type $bytes (sub $pyobj (struct
  (field $type (ref null $type))
  (field $hash i64)
  (field $len  i32)
  (field $data (ref $byte_arr)))))
```

String decision: **v1 mirrors ap-obj's storage split** (CompactAscii vs
Unicode), but with the wasm arrays chosen per width: `ascii/latin1` →
`array i8`; anything wider starts as UTF-8-in-i8 with a cached `char_count`
exactly like ap-obj (`str_repr.rs`), and `$str16`/`$str32` (UCS2/UCS4, the
full CPython trichotomy) are declared now but only emitted when the O(n)
UTF-8 indexing shows up in a corpus profile. Rationale: spec 07 must define
the round-trip of all three CPython internal encodings (orchestrator 03.01),
and declaring the subtypes up front means the widening is a codegen-local
change, not a representation break. Byte-exact `repr`/`str` come from the
shared float/str-repr ALGORITHMS (ap-obj `float_repr.rs`, `str_repr.rs`)
recompiled into the wasm runtime prelude (§6) — never re-derived in the host.

```wat
;; ---- containers ---------------------------------------------------------
(type $list (sub $pyobj (struct
  (field $type (ref null $type))
  (field $len  i32)
  (field $cap  i32)
  (field $items (ref $obj_arr)))))      ;; (array (mut (ref null $pyobj)))
(type $tuple (sub $pyobj (struct        ;; immutable: built once via array.new_fixed
  (field $type (ref null $type))
  (field $items (ref $obj_arr_ro)))))
(type $dict (sub $pyobj (struct         ;; CPython compact layout, S-01-21..24:
  (field $type  (ref null $type))       ;; sparse index array + dense entries array
  (field $used  i32) (field $fill i32)
  (field $mask  i32) (field $perturb_shift i32)
  (field $index (ref $idx_arr))         ;; (array (mut i32)), DKIX_EMPTY/DUMMY/idx
  (field $entries (ref $entry_arr)))))  ;; (array (mut $dict_entry)); entry =
                                        ;; struct{hash i64, key, value (ref null $pyobj)}
(type $set (sub $pyobj (struct ...)))   ;; same probing machinery, no values;
                                        ;; frozenset = distinct $type singleton over $set
                                        ;; (Value::Frozen arm, S-01-26, C-26 sorted repr)
```

The dict layout is a direct port of ap-obj's compact dict (index table +
dense entries, 2/3 load, perturb probing `i*5+perturb+1`, growth `used*3` →
pow2) — insertion order and key-collapse semantics (S-01-21/23) must be
byte-identical, so the ALGORITHM is fixed by the native implementation and
only the storage representation is new.

```wat
;; ---- callables / closures ------------------------------------------------
(type $code (struct                     ;; one per FnDef; module data, immutable
  (field $name     (ref $str))          ;; frame name for tracebacks, S-02-38
  (field $nparams  i32) (field $flags i32)  ;; posonly/vararg/kwonly/kwarg bits
  (field $param_names (ref $obj_arr))
  (field $func_idx i32)))               ;; index into the wasm function table
                                        ;; (uniform wasm signature, §3)
(type $function (sub $pyobj (struct
  (field $type (ref null $type))
  (field $code     (ref $code))
  (field $defaults (ref $obj_arr))      ;; evaluated at def time, S-02-7
  (field $kwdefaults (ref null $dict))
  (field $cells  (ref $cell_arr))       ;; closure env: (array (ref $cell))
  (field $globals (ref $dict)))))       ;; module namespace
(type $cell (struct (field (mut $v (ref null $pyobj)))))
;; shared mutable cells = closure capture (S-02-12); late binding falls out.

(type $method (sub $pyobj (struct       ;; bound method
  (field $type (ref null $type))
  (field $self (ref $pyobj))
  (field $fn   (ref $function)))))

;; ---- type objects + method tables ----------------------------------------
(type $type (sub $pyobj (struct
  (field $type   (ref null $type))      ;; metatype (usually `type`), S-03-2
  (field $name   (ref $str))
  (field $mro    (ref $type_arr))       ;; C3 linearization, PRECOMPUTED at build
  (field $slots  (ref $slot_arr))       ;; method table: (array (mut (ref null $function)))
  (field $bases  (ref $obj_arr))
  (field $dict   (ref $dict)))))        ;; class __dict__ (user attributes)
```

**Method table (vtable) layout:** `$slots` is indexed by *slot id* — a dense
enum of the special methods ap-obj dispatches (`__call__`, `__getitem__`,
`__add__`, `__eq__`, `__iter__`, ...; the full table is enumerated once in
spec 07 and shared with the native backend's dispatch enum). Slot resolution
walks the precomputed `$mro` array at class-build time and fills inherited
entries eagerly, so a runtime special-method dispatch is exactly
`struct.get $slots[slot]` — no MRO walk at call time. User attributes not in
the slot enum live in `$dict` (slow path, same order as ap-obj: data
descriptors → instance dict → non-data descriptors, S-03-9/10).
(p2w-precedent: p2w emits per-class dispatch tables with eagerly resolved
entries; the slot-id table itself is ours, keyed off ap-obj.)

**Instances:** user-class instances are ONE generic struct:

```wat
(type $instance (sub $pyobj (struct
  (field $type  (ref null $type))       ;; dynamic class
  (field $attrs (ref null $dict)))))    ;; lazy: null until first user attr set
```

with `__slots__`-style per-class field layouts deferred to a post-v1
optimization (documented in spec 07 as a known non-goal; correctness first).

## 3. Calling convention

All Python-level wasm functions share one signature:

```wat
(func $pycall (param $fn (ref null $function)) (param $args (ref $obj_arr))
              (param $kwargs (ref null $dict)) (result (ref null $pyobj)))
```

Body: bind params per S-02-8..S-02-10 (arg-evaluation order and binding
error text are ap-run's algorithm, ported), then the lowered body. Calls
that the lowering PROVES are static (direct call to a module-level def with
no decorator/metaclass interference) compile to a direct `call $f_N` with
positional locals — the generic convention is the fallback, not the fast
path. Raising an exception inside any function = `throw $tag_pyexc` (§5);
there is no error-value convention (the EH proposal removes the need for
the p2w-style `(result value, is_error)` pair — p2w targets browsers where
EH availability was still uneven; wasmtime ≥47 has it by default).

## 4. Bigint strategy — DECISION: **struct-based, in-module**

Two options were on the table (orchestrator): *imported runtime* (host
functions for bignum ops) vs *struct-based* (`$bigint` structs + routines
compiled into the module).

**Chosen: struct-based.** Reasoning:

1. **Artifact portability is a locked goal.** master.md ships "two
   artifacts: native binary + separate .wasm"; leaf 03.06 runs the module in
   a browser. Host-imported bignum makes the .wasm non-self-contained and
   ties arithmetic semantics to whichever host implements the imports —
   byte-exactness then has two implementations to keep in lockstep.
2. **Boundary cost + GC rooting.** A host import receives GC refs through
   wasmtime's rooted-handle API; every bigint add would pay a host-call +
   rooting ceremony. Struct-based ops stay inside the engine's collector.
3. **Byte-exactness source.** The limb arithmetic, `hash(int)` modulus
   (ap-obj `hash.rs`), and `int.__repr__` digit algorithm are transcriptions
   of CPython that already exist and are corpus-proven in ap-obj. They
   compile to wasm32 as pure Rust (num-bigint is `no_std`-friendly); the
   prelude (§6) links the SAME crate versions the native backend uses, so
   "same semantics" is enforced by shared source, not by parallel ports.
4. **p2w-precedent:** p2w implements arbitrary-precision integers entirely
   in-module (its runtime library phase), with only character-level I/O
   imported. Its mapping document shows the in-module approach passing its
   104-program suite.

Consequence for the ap-host import surface: the orchestrator's "runtime
helper imports (bignum, format, repr)" NARROWS — bignum is **not** an import;
`format`/`repr` are prelude functions too (they build `$str` GC structs,
which is awkward across the host boundary for zero benefit). ap-host's
imports become: WASI (stdio/clock/args/env) + the small `aprt` service
surface in §7. This deviation is recorded here and in the 03.05 brief; it
does not re-litigate a master.md locked decision (master.md locks the two
artifacts and the crate split, both preserved).

Small-int fast path: the `$int` kind tag lets `add/sub/mul` on two smis
compile to `i64` ops with an overflow check that promotes to the bigint
routine on i61 escape — exactly ap-obj's `Int::Smi` promotion boundary
(C-13: no wrapped result may escape; corpus sweeps `2^60 ± k`).

## 5. Exception mapping — Wasm EH (`exnref`), wasmtime ≥ 47 (≥ 37 behind a flag)

```wat
(tag $tag_pyexc (param (ref $exception)))
(type $exception (sub $pyobj (struct
  (field $type    (ref null $type))     ;; the exception CLASS
  (field $args    (ref $tuple))
  (field $tb      (ref null $tb_frame)) ;; traceback head (innermost)
  (field $context (ref null $exception));; implicit chaining, S-02-34
  (field $cause   (ref null $exception));; raise ... from ...
  (field $suppress_context i32))))
(type $tb_frame (struct
  (field $next     (ref null $tb_frame))
  (field $filename (ref $str)) (field $funcname (ref $str))
  (field $lineno   i32) (field $col i32) (field $end_lineno i32))))
```

- `raise e` → build `$exception` (args tuple, chain fields per S-02-27/28/34),
  `struct.set $tb` from the frame's line marker (`Stmt::Line`), then
  `throw $tag_pyexc`.
- `try/except` → `try_table` with `(catch $tag_pyexc $L)`; the handler
  receives `(ref $exception)` and does ap-obj's `except`-match algorithm
  (single class, tuple of classes, `issubclass` walk over precomputed MROs).
- `finally` → `try_table (catch_all_ref ...)` rethrow pattern; every exit
  path including EH ones runs the finalizer (S-02-33). `break/continue`
  through `finally` are lowered as they are in ap-run (pending-jump values),
  NOT via EH.
- Tracebacks: frames are pushed by the lowered code itself (each function
  prologue records filename/funcname into a frame-context global chain), so
  an exception escaping to the top level carries the full CPython-format
  traceback data. **Rendering** ("File ..., line N, in f" + source-line
  echo) happens at the host boundary in ap-host (it must match stderr bytes
  exactly; the source lines come from a per-module source-map data section).
- exnref payload CAN carry GC refs (wasmtime builds exception objects on the
  GC heap — Bytecode Alliance EH article) — that is precisely why the Python
  exception travels as `(ref $exception)` in the tag payload and not as an
  i32 index into a side table.
- Version constraint recorded: emitted EH modules require **wasmtime ≥ 37
  with `wasm_exceptions(true)`, ≥ 47 by default**; ap-host pins 47. Pre-EH
  fallback (error-value convention) is explicitly NOT implemented — two
  calling conventions would double the codegen surface for zero corpus gain.

## 6. Generators — explicit state structs (p2w-cited design)

```wat
(type $generator (sub $pyobj (struct
  (field $type   (ref null $type))
  (field $fn     (ref $function))       ;; code + cells to resume with
  (field $state  i32)                   ;; 0=suspended-at-entry, N=yield site N,
                                        ;; -1=exhausted, -2=closed
  (field $locals (ref $obj_arr))        ;; (array (mut (ref null $pyobj))) spilled frame
  (field $value  (ref null $pyobj))     ;; last sent / to-be-yielded value
  (field $exn    (ref null $exception)))));; pending throw-in (gen.throw)
```

Lowering: a generator function's body is compiled to ONE wasm function with
a `block/br_table` dispatch on `$state` (the classic state-machine switch,
p2w-precedent: p2w lowers `yield` to numbered suspension points resumed by a
dispatch table). Each `yield` = store live locals into `$locals`, set
`$state`, return the yielded value. `send/next/throw/close` follow S-02-21..
S-02-25 (first-send-None rule, StopIteration plumbing, close → GeneratorExit
throw-in). No stack switching, no asyncify: the corpus (incl. T2 pytest's
yield-based hooks, issue #7) needs correct suspension semantics, and the
switch machine is exactly what the native executor's generator frames
already implement — same algorithm, struct instead of heap frame.
Wasm stack-switching (`cont`/`resume`, experimental in wasmtime 47 on x86_64
Linux only) is recorded as a future performance path, not a v1 dependency.

## 7. The `aprt` host import surface (versioned)

```
(import "aprt" "abort_msg"    (func (param (ref $str))))          ;; unrecoverable internal error
(import "aprt" "now_ns"       (func (result i64)))                ;; monotonic ns (time.perf_counter)
(import "aprt" "wall_ns"      (func (result i64)))                ;; time.time
```

Everything else is WASI p1 (`wasi_snapshot_preview1`: fd_write/fd_read,
args, env, clock) — the orchestrator's "stdio, clock, args, env-lite".
The surface carries a version global exported by the module
(`aprt_version`); ap-host refuses modules from a future incompatible
version with a clear diagnostic (03.05: "import surface finalized and
versioned"). stdout/stderr byte-exactness: print/repr/format all build
`$str` structs in-module and hand bytes to `fd_write` — the host never
reformats Python values.

## 8. GC story — who owns object lifetime

**Decision: inside a wasm instance, the ENGINE's WasmGC collector owns every
Python object's lifetime. MMTk (ap-gc) never runs in wasm mode.**

- The native backend's `Heap` trait (`alloc`, `gc_trigger`,
  `add_root_range`) is backed by ap-gc → MMTk (`docs/gc-mmtk-swap.md`:
  MarkSweep plan, non-moving, explicit roots). The wasm backend needs no
  `Heap` implementation at all: allocation is `struct.new`/`array.new`,
  collection is the engine's (wasmtime: Cheney semi-space, copying,
  no barriers; V8 in the browser: generational). The ap-obj *semantics*
  that GC exposes — collection timing unobservable except via `__del__` /
  weakrefs — are what must match, and they mostly do:
  - **Cycles**: both collectors trace; reference cycles die. ✔
  - **`__del__`**: ap-gc runs finalizers at collect time with "timing not
    guaranteed" (already a documented divergence, architecture.md). WasmGC
    has NO finalization API in wasmtime 47 → `__del__` is never called in
    wasm mode. **Recorded as Class-X divergence** for the wasm target
    (pre-Wasm-era standard is works-best-effort per Amendment A-3; this one
    is semantic, so it gets a tracked issue, and the spec-07 gap list).
  - **weakref**: needs "target died" observation. With a copying collector
    and no weakrefs/finalization in the engine, an in-module weak table
    cannot work (the entry itself keeps nothing alive, but nothing tells it
    the target moved/died; host-side tables rooted via `OwnRooted` KEEP the
    object alive, breaking weak semantics). **v1 wasm mode: `weakref`
    module raises a clear compile-time import refusal** (same treatment as
    C-extensions in v1). Revisit when wasmtime lands the weak-references /
    finalization GC extensions; spec 07 records the watch item.
- **Cross-boundary rooting**: the only place the two worlds touch is ap-host
  holding GC handles (e.g. an `OwnRooted<StructRef>` for a cached entry
  point, or a caught `(ref $exception)` after the instance unwinds). Those
  handles are rooted in the wasmtime Store scope; the engine updates them
  across collections. Rust-side values they reference (none in v1 — `$str`
  payloads are copied out as bytes for stderr rendering) never alias MMTk
  memory. Rule: **ap-host must never retain a GC handle across a new
  instantiation, and never store wasm GC refs in native ap-obj values.**
- **Linear memory vs GC heap**: linear memory holds only byte buffers
  (I/O scratch, source-map/data sections, `array i8` payloads are GC-heap).
  No Python object identity is ever derived from a linear-memory address.

## 9. Threading note

wasmtime **47 removed wasi-threads** (#13558). The orchestrator's contract
("wasi-threads where the host provides them; single-threaded fallback with a
startup diagnostic") therefore resolves in v1 to: **single-threaded, with a
startup diagnostic emitted when the module requests threads** (`threading`
import → diagnostic + clear error). 03.06's threading spike evaluates the
shared-everything-threads proposal instead of wasi-threads, and documents
the result either way (feasibility probe, not a gate). This is a
target-dependent guarantee documented in the master contract sense, not a
hidden difference.

## 10. Module skeleton (what ap-codegen-wasm emits, in order)

1. type section: `$pyobj` hierarchy above + function types
2. import section: WASI p1 + `aprt` (§7)
3. function/table sections: one wasm func per `$code` (uniform convention §3
   for dynamic, specialized signatures for provably-static calls),
   `$pycall` table for `call_indirect`
4. globals: singletons (`None/True/False/Ellipsis`), rt-init flag
5. memory (1 page min) + data sections: string/bytes literals (as raw bytes
   copied into `$byte_arr`s at init), source map for tracebacks
6. `(func $rt_init)`: allocate singletons + builtin type objects (with
   precomputed MROs and slot tables), interned strings
7. `(func $_start export)`: `$rt_init` (guarded), then the module body items
   in source order; uncaught `$exception` at this boundary → ap-host renders
   the CPython-format traceback to stderr and sets exit code 1
8. custom section `aprt_version`

Hello-world skeleton status: `crates/ap-codegen-wasm` + `crates/ap-host`
currently carry the pre-GCWAS bootstrap (linear-memory `print` via
`fd_write` + wasmtime/WASI runner + integration test) landed with this
design wave — it proves the ap-ir → bytes → wasmtime → captured-stdout
pipeline end to end; the GC struct hierarchy above lands in 03.01/03.02.

## 11. Open questions for spec 07 (leaf 03.01)

- Exact slot-id enum (must be shared with native dispatch; frozen there).
- Interning policy for `$str` literals (data-section bytes vs rt_init
  allocations — affects module size vs startup, 03.05's <50ms target).
- `float.__repr__` shortest-round-trip (ap-obj `float_repr.rs`) ported to
  prelude Rust vs emitted wasm — same-source rule says prelude Rust.
- f-string `FORMAT_VALUE` spec surface (docs/fstring-differential.md).
- Set/frozenset repr ordering (C-26: sorted) — algorithm port, not storage.
