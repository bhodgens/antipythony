# pon Study — AST→IR Lowering (Plan 02 input)

Source: github.com/can1357/pon, clone at commit `d0a171772341178570f42422f10bee436c4f8c50`
(shallow-50). Dual-licensed MIT/Apache-2.0, granted to us — read/adopt/adapt freely,
attribution headers required before any adapted code lands. All pon paths below are
relative to the repo root and were verified to exist in this clone.

## Overall shape

pon's frontend (`pon-ir`) is the closest published analogue to our ap-parse → ap-ir
pipeline. Layout:

- `pon-ir/src/parse.rs` — ruff parse, pinned 0.14.0, PY314.
- `pon-ir/src/lower.rs` (2332 L) — the driver: scope analysis first
  (`pon-ir/src/lower/scope.rs`, 1657 L), then per-family modules under
  `pon-ir/src/lower/`: `assign.rs`, `class.rs`, `comprehension.rs`, `control.rs`,
  `func.rs`, `generator.rs`, `import.rs`, `match_.rs`, `mangle.rs`, `strings.rs`,
  `synth.rs`, `try_.rs`, `with_.rs`.
- `pon-ir/src/ir.rs` (816 L) — one CFG IR, everything boxed/opaque; `InstKind`
  ops; `FeedbackSlot` per specializable inst; `CellId`/`LocalId`/`NameId` u32 ids.
- `pon-ir/src/desugar.rs` — explicit identity pass (placeholder slot).

**Key structural difference vs ap-parse:** pon lowers ruff AST directly to a flat
CFG IR (basic blocks + terminators) with Python semantics resolved at lower time.
Our `crates/ap-parse/src/lowering.rs` (1489 L) lowers to a structured, source-shaped
ap-ir (`crates/ap-ir/src/item.rs`, `expr.rs` — Stmt/Expr trees, no blocks) with scope
classification emitted alongside (`ScopeInfoModule`). pon's lowering is effectively
our Plan 02 "structured IR → CFG" step fused into the frontend. That is the single
biggest architectural decision to weigh: pon's choice means try/finally edge
rewriting and generator state-machine transforms happen on IR; ours defers all of
that to ap-codegen-native. Both are defensible; pon's is further along and battle-
tested by 244-module byte-exact conformance (`conformance-floor.json`).

## Scoping model

- pon: `pon-ir/src/lower/scope.rs` — full pre-pass symbol table. `NameClass` = Local /
  Global{explicit} / Free{slot} / Cell{local_slot, cell_slot} / Builtin
  (`scope.rs:31-46`). Cell assignment: names needed by children become `cell_vars`
  in BTreeSet order; class scopes claim the implicit `__class__` cell with **no
  symbols entry** — replicating CPython `ste_needs_class_closure`
  (`scope.rs:144-152`, `scope.rs:1095-1124`). Parameters (incl. `*args`/`**kwargs`)
  participate as formals for cell capture (`scope.rs` tests ~1346). `ParameterSummary`
  records positional-only / vararg / kwonly / kwarg shape in ABI order
  (`scope.rs:96-126`).
- us: `ScopeData` cells/frees/freevar computation in `lowering.rs:175-345`,
  classified per S-02-1..S-02-13; verified byte-matched to CPython symtable
  (AGENTS.md status). Equivalent classification, but pon additionally assigns
  **numeric local/cell slot orders** at lower time, which Plan 02 needs anyway for
  stack frames. **Adopt:** slot-numbering rules (BTreeSet ordering of cell_vars, cell
  order = capture order) as the canonical slot layout — determinism matters for
  differential debugging of frames.

## Closures / cells

- pon: `CellId` instructions `LoadCell`/`StoreCell`/`DeleteCell` in the IR
  (`pon-ir/src/ir.rs`; use sites in `lower/assign.rs:73-190`). Free vars of a child
  resolve to parent cells at lower time; class bodies only support the implicit
  `__class__` cell — any other class-body cell is `Unsupported`
  (`lower/class.rs:14-19`) — a deliberate restriction we cannot copy.
  Runtime cell = one-pointer `PyCell` (`pon-runtime/src/types/cell.rs`).
  `__classcell__` protocol: class body stores the cell object into the namespace;
  `__build_class__` pops and fills it (`lower/class.rs:60-70`).
- us: ScopeInfoModule carries cells/free names but ap-ir has no cell ops yet —
  Plan 02 must add them (or equivalent frame-slot+cell indirection).
  **Adopt:** the `__classcell__` namespace-store protocol exactly (it is what makes
  `super()` in methods and zero-arg `super()` work without special cases).

## Exception representation

- pon: structured exception records in the IR, notlanding/unwind. `PushExcInfo
  {target, stack_depth, kind}` / `PopExcInfo` / `MatchExc` / `Reraise`
  (`lower/generator.rs:60-90` shows the async-for pattern; `pon-runtime/src/abi.rs:651-672`
  `HandlerInfo {frame, target, stack_depth, kind}`). A `Terminator::RaiseTerm`
  marker is emitted first and **retroactively redirected** to the innermost handler
  block (`lower/try_.rs:30-50`, `redirect_raise_terms`) — raises are ordinary edges,
  no unwinding anywhere in the runtime; the AoT entry wraps Rust panics with
  `catch_unwind` only as a safety net (`pon-runtime/src/aot_entry.rs:53`).
- try/finally: every protected phase gets **departing-edge trampolines** —
  break/continue/return edges pop the phase's handler record and inline a copy of
  the `finally` body under the OUTER loop/return context; nested trys chain
  (`lower/try_.rs:38-75` doc comment, `PhaseExits`). `return` parks its value in a
  shared return slot and jumps the trampoline chain (`lower/control.rs:30-43`).
  `async for` advance is wrapped in a PushExcInfo region whose handler matches
  `StopAsyncIteration` and reraises the rest — protected region covers ONLY the
  advance (`lower/generator.rs:66-100`).
- us: ap-ir still has structured Try/With statements; nothing lowers them to edges
  yet. **Adopt:** (1) the RaiseTerm-then-redirect pattern — emit `raise` as a
  dedicated terminator during body lowering, fix targets in one pass; (2) the
  trampoline-per-exit-edge finally discipline with value parking; (3) pin the
  "PopExcInfo on every exit edge of a protected region" invariant as a Plan 02 IR
  verifier rule. Note pon references internal pins ("J0.6 §3.1") in comments —
  those pin docs are not in the clone; the code is the authority.

## Generators / async

- pon: `yield`/`yield from`/`await` lower to marker insts (`Yield`, `YieldFrom`,
  `Await`); a post-lowering transform `transform_generator_function`
  (`lower/generator.rs:225-270`, called from `lower.rs:1517`) splits each suspend
  point into `Terminator::Suspend{state, val, resume}` with dense state numbers,
  prepends a `GenResumePayload` to entry (so `throw()` on a never-started generator
  raises before user code), and **spills every cross-block SSA value to a temp
  local** so codegen can whole-frame spill/reload at suspend points. `yield from`
  and `await` both expand to a delegation loop around one `GenDelegateStep` state
  (`lower/generator.rs:24-64`). The generator object is stackless: heap `PyGenerator`
  + heap `PyFrame{state, locals}` (`pon-runtime/src/types/generator.rs:1-60`,
  `pon-runtime/src/types/frame.rs:1-40`); resume = ordinary call to the compiled body.
- us: ap-ir keeps async as plain bodies. **Adopt:** the whole design — it is clean
  and directly portable: (1) lower suspend markers first, transform later; (2) dense
  state numbering with a debug assertion pass (`debug_assert_suspend_numbering`);
  (3) the cross-block-spill rule, which makes tier-1 OSR and generator frames share
  one mechanism; (4) `GenResumePayload` entry guard. This kills any need for
  stack-switching coroutines in Plan 02.

## Match lowering

- `lower/match_.rs` (408 L): per case, "test, bind, fall through" — predicate insts
  branch to the case's fail block on mismatch; **capture bindings store eagerly and
  leak partial binds from failed cases**, matching CPython exactly (documented at
  `match_.rs:1-7`). Guards evaluate only after full pattern match. Subject evaluated
  once; `done_block` shared. Captures validated against a BTreeSet to reject
  duplicate bindings (`validate_case_pattern`).
- us: ap-ir has a structural `MatchCase`/`Pattern` tree (lowering.rs:562-581).
  **Adopt for Plan 02:** the eager-partial-bind rule and the test-chain shape when we
  lower patterns to CFG; pon's file is a near-direct template for that leaf.

## f-strings / t-strings

- `lower/strings.rs`: f-strings lower to `BuildString{parts}` with
  `FStrPart::Interp{value, conversion, format_spec}`; t-strings to `BuildTemplate`.
  Critical subtleties captured: (1) walk implicit-concatenation **parts**, not
  `elements()` — ruff's `FStringValue::elements()` silently drops plain literals
  mixed into concatenation (`strings.rs:11-18`); (2) debug-text `f"{x=}"` emits the
  literal text then defaults conversion to `r` when no format spec
  (`strings.rs:55-75`); (3) `TEMPLATE_LITERAL_CONVERSION = u8::MAX` sentinel for
  t-strings. Nested format specs recurse through the same path.
- us: `lower_fstring_value`/`lower_interp_element` (`lowering.rs:1044-1114`) already
  produce `FStrPart` with conversions; ap-ir's `Conversion` enum should be checked
  against pon's u8 wire form (0/`r` default/`u8::MAX` sentinel) when we pick a
  runtime repr. **Adopt:** the parts-not-elements ruff gotcha — we may have the same
  bug; worth a differential test with `'"#define X {" f"{cname};" "%s"'` style mixes.

## Decorators / defs / PEP 649

- `lower/func.rs:13-52`: annotations get a synthetic `__annotate__` child scope keyed
  to the def's span (PEP 649 lazy annotations); entries ordered posonly → args →
  `*args` → kwonly → `**kwargs` → return (`func.rs:56-90`). Decorators apply in
  reverse iteration as `Call` insts (`func.rs:42-46`). Classes: the namespace
  `__annotate__` is claimed up front but stored at body END, matching CPython
  (`lower/class.rs:44-52`); type-param thunks and `__type_params__` handled in
  `lower/synth.rs` (`synthesize_type_alias_thunk`, `emit_type_param_prologue`).
- us: ap-ir carries decorators on `FnDef`/`ClassDef` and annotations as `Ann`, but
  there is no `__annotate__` lowering yet. **Adopt:** the PEP 649 synthesis pattern
  (child-scope + span-key disambiguation + store-at-body-end for classes) when
  Plan 02/03 need `__annotations__` semantics; also pon's name-mangling pre-pass
  (`lower/mangle.rs:1-24`) — an AST transformer run before scope analysis so
  mangling and scoping see consistent spellings; our lowering does not mangle yet.

## Comprehensions

- `lower/comprehension.rs` (259 L): comprehensions lower as synthetic functions
  named `<listcomp>`/`<setcomp>`/`<dictcomp>`/`<genexpr>` with the implicit `.0`
  iterator parameter, nested-generator shape for genexprs (asserted at
  `lower.rs:1885,2253`). Matches CPython scope behavior by construction.
- us: ap-ir keeps comprehension clauses structured (`CompClause`). Plan 02 will need
  the synthetic-function shape; pon's naming (`<listcomp>`, param `.0`) is also what
  tracebacks/qualnames expect — keep the same spellings.

## What to skip

- pon's `unsupported_at` walls (class-body cells, some dynamic code) are pon-scoped
  gaps, not designs.
- `pon-ir/src/lower.rs` Phase-A/B migration narrative — irrelevant to us.
- No README/code contradictions found. Minor caveat: README says "tagged small-int
  fast path is landing" while `pon-codegen/src/optimizing.rs` already compiles
  `IntI64` fast-path regions — README understates; the code is ahead.

## Direct adoption checklist for Plan 02 leaves (with attribution headers)

1. Cell-slot layout + `__classcell__` protocol (scope.rs, class.rs).
2. RaiseTerm-redirect + finally-trampoline edge discipline (try_.rs, control.rs).
3. Generator state-machine transform incl. cross-block spill rule (generator.rs).
4. Match test-chain with eager partial binds (match_.rs).
5. Mangle-as-AST-predpass (mangle.rs); PEP 649 `__annotate__` synthesis (func.rs,
   synth.rs, class.rs).
6. Ruff f-string parts-not-elements gotcha test (strings.rs).
