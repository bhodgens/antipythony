# 02-calls-and-attrs — Call/LoadAttr/Subscript via shared helpers

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-calls (branch
tier1-fp-calls, from current main). Own commits, explicit pathspecs.
ONE heavy cargo lane; CARGO_BUILD_JOBS=4; CARGO_TARGET_DIR=$PWD/
target-fp-calls everywhere.

Parent: plans/fast-path/master.md. DEPENDS: 01-core/01-control-flow
MERGED. ~90K context.

## Deliverable

1. **Call opcode execution**: route through the SAME call_value path
   the tree-walker uses (semantics single-source). Optimization scope:
   the CALL PREP (receiver resolution, arg stack marshalling) — not
   the callee's semantics. Method-call fast path for
   (native-receiver, known-method) pairs may consult native_attr.
2. **LoadAttr / StoreAttr opcodes**: route through attr_get/attr_set
   helpers. Subscript opcodes through the getitem/setitem helpers
   (int-needle semantics from the a3f53fd fix ride along free).
3. **Comprehension lowering**: ListComp/SetComp/DictComp/GeneratorExp
   — lower comprehensions to an inline loop shape (NO separate
   function frame; the comprehension's implicit function scope is
   emulated with a slot namespace). Generators stay on the
   tree-walker (out of scope per master.md).
4. **Keyword args, *args/**kwargs, defaults**: lower to the existing
   call machinery; unsupported exotic shapes fall back per-function.
5. **Coverage gate additions**: method calls on all native types
   (bytes/str/int/list/dict surface families), nested calls, kwargs,
   defaults, comprehension shapes, subscript chains, attribute chains.

## Targets

- dict.getitem/get rows in bytecode mode: measurable improvement over
  tree-walk (the 190ns walk floor was the dominant term; the bytecode
  step removes it — target >= 2x vs tree-walk on dict loops).
- str method-call loops: >= 2x vs tree-walk.
- All byte-exactness gates green in both modes.

## Files you own

crates/ap-bytecode/{lower.rs, interp.rs}, Instr extensions,
fix_bytecode.py probe extensions.

## Self-verification checklist
- [ ] Call/LoadAttr/StoreAttr/Subscript opcodes three-way identical
- [ ] comprehension shapes (list/set/dict/genexpr-fallback) covered
- [ ] kwargs/defaults/exotic-arg fallbacks loud (APX_BP_DEBUG)
- [ ] dict/str method-call loops >= 2x vs tree-walk
- [ ] standard battery green
