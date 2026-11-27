# Leaf WASM-SUPER — zero-argument super() on the wasm backend (+ the call-path defects it exposed)

**Worktree:** `~/.git/worktrees/leaf-wasm-super` (branch `fix-wasm-super-zeroarg`
off main@606926b)
**Date:** 2026-11-27
**Input:** bughunt 2026-11-26 finding D7 (`.hermes/audits/2026-11-26-bughunt-addendum.md`
section 6.1): zero-argument `super()` always raises
`RuntimeError: super(): no arguments` on the wasm backend. User decision
2026-11-27: "do B" = implement it (Option B), not keep the gap.

## What this leaf fixes (6 defects, one call path)

`super().__init__()` is a chain of four steps. Each step had a defect, so a fix
for the first step alone ended in a trap at the next step. The probes below
isolated each one.

| # | Defect | Site | Base behavior |
|---|---|---|---|
| 1 | Zero-argument `super()` had no start class and no `self` | `lower.rs` `emit_call` super arm | `RuntimeError: super(): no arguments` |
| 2 | A method overridden in a subclass was bound into the BASE class too (bughunt D4): the function slot was found by NAME with `rposition`, i.e. the last `def` of that name in the module | `lower.rs` `Stmt::LocalDef` slot lookup | `A().g()` ran the body of `B.g` |
| 3 | Attribute lookup on a `super` object searched the class named `super` (the comment in `super_new` recorded this walk as "deferred") | `prelude_classes.rs` `attr_get` tail | trap: the null result was cast to `$function` |
| 4 | An attribute call used the RECEIVER as the first argument. For a `super` object that passed the proxy instead of the instance; for a class object (`A.m(b)`) it added the class and overflowed the arity | `lower.rs` attribute-call path | wrong object mutated; `A.m(b)` trapped in `py_call` |
| 5 | An attribute call with more than 2 arguments hit an early `return` with two operands left on the stack | `lower.rs` `if n > 2 { return }` | `A().m(1, 2, 3)` trapped at run time |
| 6 | No place on a function object held its defining class | `types.rs` `$function` | n/a (new field) |

## Design

CPython fills the two hidden arguments of `super()` from the enclosing method:
the class where the method is DEFINED (`__class__`) and the first argument.
The bytecode interpreter of this repo does the same with a method-context
stack (`interp.rs` `builtin_super`, zero-argument arm). The wasm backend now
carries the same two facts:

- **`$function.owner`** (new field 6, `F_FN_OWNER`): the defining class, or
  null for a plain function. The class body sets it when it binds a method
  (`emit_function_object` reads `ctx.cls_obj`). A `def` nested inside a
  method is emitted later with `cls_obj = None`, so it gets no owner and its
  bare `super()` keeps raising `RuntimeError` (the c021 observable — that
  case matches CPython and was NOT changed).
- **`super_mro_get`** (new prelude fn 74): the CPython lookup rule for a
  `super` object. Find the start class in the MRO of `type(obj)`, then search
  each class AFTER it for the name.
- **`bound_obj`** (new prelude fn 75): which object an attribute call binds as
  its first argument. A `super` object binds the instance it stands for
  (`__ap_obj__`); a class object binds nothing (the args already carry the
  instance); anything else binds itself.
- A `def` is identified by name AND `def_line` (new `UserFn.def_line`, taken
  from the IR `FnDef.def_line`), so two methods with the same name in two
  class bodies no longer collide.

`FN_FIRST_USER` moves 74 -> 76 (two new prelude functions). The index table is
dense and append-only, per the existing convention.

## Baselines (probe-first, on the base code)

All probes ran through the real library path (`ap_parse::lower_source` ->
`ap_codegen_wasm::lower_module_ex` -> `ap_host::run`).

**My 4 new corpus cases on BASE code** (final case bodies; base worktree
`leaf-wasm-super-base` @ 7f6a67f, its OWN target dir `target-base`; the log's
`Compiling ap-codegen-wasm (...leaf-wasm-super-base/crates/ap-codegen-wasm)`
line proves the binary is base code):

`classes/exceptions wasm differential: 17 passed, 4 failed`, cargo exit 101.
The 17 existing cases pass on base. Exactly my 4 new cases fail:

| case | base result (exact) | the defect it catches |
|---|---|---|
| CLW009 | rc 1, no stdout, `RuntimeError: super(): no arguments` | #1 (D7) |
| CLW010 | rc 1, no stdout, `RuntimeError: super(): no arguments` | #1 (D7), #2 |
| CLW011 | rc 1, no stdout, `RuntimeError: super(): no arguments` | #1 (D7), #2 |
| CLW012 | wasm trap, first frame `wasm function 37` (`py_call`) | #3, #4 |

Same 4 cases on the leaf: all pass (see Results). Log:
`~/.git/worktrees/leaf-wasm-super-base/base-gate-final.log`.

**Attribute-call shapes on BASE code** (main-checkout CLI, built 2026-11-27
00:27, before this leaf):

| program | base result | CPython 3.14.7 |
|---|---|---|
| `A().m(1, 2)` | `3` | `3` |
| `m(1, 2, 3)` (plain function) | `6` | `6` |
| `A().m(1, 2, 3)` (method) | trap in fn 16 | `6` |
| `super(B, self).f()` | trap in fn 37 | `A` |
| `A.__init__(self)` | trap in fn 37 | works |

## Results (leaf code, first-hand)

**Differential gate** `cargo test -p ap-codegen-wasm --test classes_exceptions_wasm_gate`:
`test result: ok. 1 passed; 0 failed`, cargo exit 0. The gate runs every `.py`
in `classes_wasm` and `exceptions_wasm`: 17 existing + 4 new = 21 cases, each
byte-exact against its CPython 3.14.7 pin.

**Other wasm gates** (one cargo call, because the function index table and the
`$function` struct changed, which every module uses): `controlflow_wasm_gate`,
`corpus_wasm_sweep`, `cli_corpus_dump`, `--lib`: cargo exit 0, four
`test result: ok` lines, 6 passed, 0 failed.

**Isolated probes** (leaf code), all equal to CPython 3.14.7:

| probe | program | base | leaf |
|---|---|---|---|
| p1 | `super().__init__()`, same method name in 2 classes | `RuntimeError` | `A` |
| p3 | `A().who(), B().who()` (override) | `B B` | `A B` |
| p5 | `super(B, self).f()` | trap | `A` |
| r2 | `type(super().m()).__name__` | `RuntimeError` | `str` |
| t4 | `print(A.m(b))` | no output | `A.m` |
| u1 | `A().m(1, 2, 3)` | trap | `6` |

**c021 unchanged and VERIFIED:** `tests/corpus/classes/c021_super_validation.py`
is in the `classes` family (not a wasm corpus family), so the differential
gate never runs it. I ran it through the leaf code with the sweep gate's
`CASE` filter: `CASE=c021_super_validation.py cargo test -p ap-codegen-wasm
--test corpus_wasm_sweep` -> `classes/c021_super_validation.py: PASS` (the log's
`Compiling ap-codegen-wasm (...leaf-wasm-super/crates/ap-codegen-wasm)` line
confirms the leaf binary). PASS = stdout, stderr and exit code equal the
committed pin, and I re-ran the pin's source on live CPython 3.14.7: identical,
last line `RuntimeError: super(): no arguments`. So the existing error path is
intact: a bare `super()` in a nested function (no `self`) still raises
RuntimeError. The guard in code is `ScopeKind::Fn && params >= 1` plus the null
owner of a def nested in a method.

## Honest scope

- `super()` inside a `classmethod` needs decorators, which the wasm backend
  rejects at build time ("decorated def"). NOT implemented; c023 does not
  build on wasm before or after this leaf.
- Multi-base classes (diamond, c022) are rejected at build time. NOT touched.
- `super` objects are detected by CLASS NAME (`"super"`), the same test the
  existing `isinst` marker logic documents. A user class literally named
  `super` would be misrouted. This is the pre-existing convention.
- An attribute read on a `super` object that is NOT called (e.g. `print(super(B, b).__class__)`)
  goes through the new lookup; only `.name` reads were probed.
- The `n > 2` fix was proven for 3 arguments (u1, u4). 4+ arguments were not
  probed.
- One `#[ignore]`-free path changed for EVERY attribute call (the args array
  now rides `TEMP_LOCAL_OBJARR_RO` and the function rides `TEMP_LOCAL_FN`).
  The three wasm gates above cover it; the native and bytecode tiers do not
  use this code.

## Base defects found and NOT fixed (out of scope, filed below)

These trap on the BASE code with no `super()` involved, so they are not this
leaf's regression. They forced a rewrite of three of my case bodies, because a
test case must test one thing.

1. **`str + str` traps on wasm.** `print('B' + 'A')` traps in `FN_BINOP`
   (function 34) at a cast to `$int`. Even two literals fail. Base probes
   s1..s4, r4..r6. CPython: `BA`. This is a large gap: it likely blocks most
   real programs on wasm. **Recommended next leaf.**
2. **A nested `print` corrupts an outer `print`.** `print('A', inner())` where
   `inner` prints: base gives `inner` then `inB`, CPython gives `A B`. Cause:
   the one shared print buffer. Probes v1, v2.
3. **`print(A.m)` (a plain function object) traps** in fn 16. Probe t6.

## Process notes (for the next leaf author)

- **Two worktrees must never share one `CARGO_TARGET_DIR`.** The base and the
  leaf gate used the same dir here; cargo saw a fresh fingerprint and ran the
  BASE binary for the leaf run. Two "still red" results were that artifact.
  Always check the `Compiling <crate> (<path>)` line names the worktree under
  test.
- **Prelude local grids:** a function with N params needs a pad of
  (3 - 1 - N)... in practice: 2 params -> 1 pad entry puts real local 3; 1 param
  -> 3 pad entries. Copying `attr_get`'s `(2, oref())` pad into a two-param
  function shifted every index by one and cost two build cycles. Read the
  emitted `(local ...)` list in the wat dump and count by hand.
- **The host error carries no detail** ("failed to compile: function[N]").
  `wasmparser::Validator::new_with_features(default | GC |
  FUNCTION_REFERENCES | EXCEPTIONS).validate_all(&bytes)` gives the offset and
  the reason; `wasmprinter` with `start_line` offsets maps it to a line.
- **An `If(Empty)` arm must not `return` inside an expression.** A `return`
  in the attribute-call arm ended the whole user function before an enclosing
  `print`. Park the differing value in a local per arm, then share one call.
- `Br` depth inside `if` > `loop` > `block`: 0 = if, 1 = loop, 2 = block.
  `Br(1)` there jumps to the loop head and spins forever.
- `struct.get` on a `$type`'s own type field traps: that field is null by
  construction (`cls_new`). Test the shape with `ref.test` first.
