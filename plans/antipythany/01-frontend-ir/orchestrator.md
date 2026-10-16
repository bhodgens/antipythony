# Plan 01 - Frontend and IR

Goal: Python 3.14 source -> typed AST (ruff) -> ap-ir module. The IR is the trunk
both backends consume; its stability matters more than its completeness.

## Design

Two crates:

- `ap-parse`: thin wrapper over ruff_python_parser pinned to PythonVersion::PY314.
  Owns the version pin. Produces ruff AST plus source maps (byte offsets -> line/col).
- `ap-ir`: our own IR. No dependency on ruff types (convert at the boundary) so
  backends never see vendor types.

IR shape (v0 - expect revision in 01.03):

```
Module { items: Vec<Item> }        // fn, class, stmt-level defs
FnBody { blocks: Vec<Block> }      // structured, not CFG v0: stmt tree + explicit
                                   // yield/raise edges; CFG lowering is 02.02's job
Expr: literals, names, binop, unaryop, compare, call, attribute, subscript,
      lambda, comprehension, yield, await, fstring
Stmt: assign, augassign, ann, return, if, while, for, with, try, match,
      raise, assert, global/nonlocal, import, del, pass
Type lattice for specialization: Unknown | Int | Float | Str | Bool | None |
      ExactClass(DefId) - annotations seed it, tier-1 feedback refines it
```

Semantics preservation rule: lowering must not drop or reorder operations with
observable effects. Every lowering decision cites the spec-pack clause it preserves.

## Leaves

### 01.01 - ap-parse wrapper
ruff parser integration: parse file/string -> AST + source map; error recovery off
(fail with byte-precise diagnostic); fixture tests over a 50-file corpus of valid +
5 invalid programs. Expose: `ap_parse::parse(src) -> Result<ModuleAst, ParseError>`.
Test leaf: tests/01-parse-from-spec.md (diagnostic formats, valid-program acceptance).
Gate: corpus round-trips (parse -> no error) on all 50 fixtures.

### 01.02 - ap-ir data structures + builder
IR types per the shape above, plus `Builder` with clause-cited construction methods.
No lowering yet. Property tests: every AST node kind maps to at least one builder
method (exhaustiveness table). Expose: ap-ir crate, stable PartialEq on all nodes
(needed for golden tests). Test leaf: tests/02-ir-shape-from-spec.md.
Gate: builder constructs all corpus constructs; IR node kind enum covers ruff AST
node kinds (documented exhaustiveness matrix committed).

### 01.03 - AST -> IR lowering
PON LICENSE GRANTED: pre-task for this leaf is a guided study of pon's pon-ir
lowering decisions (scope classification, exception/generator lowering) with a
written findings note; adapt where the license terms allow, attribute per master
license table TODO.
Lower the full v1 subset: functions (args incl. *args/**kwargs/defaults/kwonly),
classes, comprehensions, generators, try/except/finally/else, with, match, f-strings,
decorators, imports, global/nonlocal. Scope resolution pass emits per-function
cell/free/global classification. Golden tests: 20 corpus programs -> checked-in IR
serializations. Expose: `ap_ir::lower(ast) -> Result<ir::Module, LowerError>`.
Test leaf: tests/03-lowering-from-spec.md (scope classification, edge semantics).
Gate: all 20 goldens byte-stable; scope classification matches spec clauses.

### 01.04 - IR verification pass
Static checks before codegen: name resolution complete, no unbound local read
(matches Python UnboundLocalError semantics - cite clause), match exhaustiveness
NOT required (Python semantics), constant-fold only provably-pure exprs. Expose:
`ap_ir::verify(module) -> Vec<Diagnostic>`. Test leaf: tests/04-ir-verify-from-spec.
Gate: differential run: for every corpus program, CPython runtime errors on unbound
locals are predicted by verify with matching message text.

## Dependencies
00.01 (workspace), 00.03/00.05 for semantics citations.

## Dispatch notes
01.01 and 01.02 parallel after 00 completes; 01.03 next; 01.04 last. The test leaves
for this plan dispatch as soon as 00.05 lands - they are written from spec pack
sections 01-02 + the compat contract, in parallel with implementation.
