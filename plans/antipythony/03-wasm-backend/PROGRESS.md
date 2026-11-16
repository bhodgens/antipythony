# Plan 03 - WasmGC backend: progress

## Status: DESIGN WAVE LANDED (2026-10-30, uncommitted)

- Design doc: `docs/03-wasm-representation-design.md` — pyobj WasmGC struct
  hierarchy, bigint strategy (struct-based in-module, decision recorded),
  exception mapping (Wasm EH exnref, wasmtime >= 47 pin; >= 37 behind
  flag), generator state structs, GC ownership (engine WasmGC owns object
  lifetime in wasm mode; MMTk never runs there; __del__/weakref
  divergences recorded).
- Leaf briefs: `leaves/03.01` .. `leaves/03.07` (7 leaves). Orchestrator
  Test Gate coverage matrix filled.
- Hello-world skeleton (crates were empty stubs):
  - `crates/ap-codegen-wasm`: `lower_module` (print-str-literal → WASI
    fd_write bootstrap; everything else explicit `CodegenError::Unsupported`)
    + 2 unit tests. Deps: ap-ir, wasm-encoder 0.263.
  - `crates/ap-host`: `run(bytes) -> RunResult{stdout,stderr,exit_code}`
    (wasmtime 47, WasmGC+EH flags explicit, WASI preview1 via
    wasmtime-wasi p1, MemoryOutputPipe capture) + 1 end-to-end test.
  - `cargo test -p ap-codegen-wasm -p ap-host`: see leaf-report / commit
    message when landed (design wave did not commit).

## Key decisions locked in the design wave (for orchestrator review)

| Topic | Decision | Where |
| --- | --- | --- |
| Value representation | uniformly boxed `(ref null $pyobj)`, no i31ref in v1 (ap-obj smi is i61) | design §1 |
| bool | $int singletons with `bool` type object (bool ⊂ int, S-01-9) | design §2 |
| Bigint | struct-based in-module; NOT a host import (portability, GC rooting, shared-source byte-exactness); orchestrator's "bignum import" surface narrowed accordingly | design §4, leaf 03.05 scope 3 |
| Exceptions | Wasm EH `try_table`/`throw`, tag payload `(ref $exception)`; wasmtime >= 47 pin (>= 37 behind `wasm_exceptions(true)`); no error-value fallback | design §0/§5 |
| GC ownership | engine's WasmGC collector owns object lifetime in wasm instances; MMTk never runs in wasm mode; `__del__` uncalled + `weakref` refused = tracked Class-X divergences | design §8 |
| Threading | wasmtime 47 REMOVED wasi-threads → v1 single-threaded + startup diagnostic; 03.06 spikes shared-everything-threads | design §9, leaf 03.06 |
| Generators | explicit $generator state struct + br_table state machine (p2w-cited design); no asyncify, no stack-switching in v1 | design §6 |

## Dispatch order (orchestrator.md, unchanged)

03.01 (spec 07 = the contract; unblocks T-12/T-13) → 03.02 → 03.03 →
03.04 sequential; 03.05 parallel from 03.03 onward; 03.06 then 03.07 last.


## 2026-11-07: leaf 03.02 SPLIT (orchestrator decision)

03.02 as briefed (values + arithmetic + control flow + calls + containers)
is multi-session work: byte-exact bigint limb routines, siphash13, compact
dict probing, and shortest-round-trip float repr must be emitted in-module
or as prelude Rust (spec-07 §6/§9) — an emitter restructuring, not one
leaf. Split: 03.02a (skeleton: type hierarchy, rt_init, GC-$str print),
03.02b (prelude runtime + value/arithmetic lowering), 03.02c (control
flow + calls + closures). 03.02a LANDED in private worktree
~/.git/worktrees/leaf-0302 @ 1b02ae8 (from 47be826), verified by the
orchestrator: 5/5 ap-codegen-wasm + 3/3 ap-host tests green incl.
end-to-end wasmtime hello-world; 18 named Unsupported sites, fail-closed.
NOT merged to trunk; merge with the 03.02b/c wave.
ENGINE FINDING carried into 03.02b scope: wasmtime 47 drops ACTIVE data
segments after instantiation — the emitter carries a passive duplicate
segment (index 1) for array.new_data. DEVIATION for 03.05 review:
aprt.abort_msg declared param is anyref instead of spec-07-41 (ref $str)
— wasmtime host-func matching cannot express the self-referential
concrete type; call sites still pass $str values.

## 2026-11-15: leaf 03.03 session (classes/EH slice — PARTIAL, honest state)

WEDGE ROOT FIXED (commit 35f2ace0): the prior agent's 45-min silent wedge
+ "fn[51] failed to compile across ALL modules" was exc_ctx_set routing
its struct.set OBJ through a (ref null $pyobj) local — locals are
statically their DECLARED type, so the ref.cast refinement was lost.
Fix: cast kept on the operand stack. Engine fact locked: a refinement
routed through a local slot is LOST; struct.set obj needs the cast result
stack-resident.

CORE PRELUDE LANDED (5d26110e): FN_EXC_NEW..FN_TB_RENDER (52..60):
mro_dict_get (MRO dict walk), exc_new ($excinst w/ args tuple), cls_new
(single-base C3), instance_new/attr_get/attr_set (S-03-9/10 v1), cls_call
(BaseException-derived -> exc instances w/ e.args; plain -> __init__
static-arity 0..=3 cascade), call_of, tb_render (CPython-format traceback
to fd 2, single-frame v1). types.rs: $excinst(35) sub $exception
($exception made non-final); FTs 93..97; FT ids bumped +1 for the rec
growth. rt_init builds the EXC_TREE (24 classes) with real MRO chains.

COMMITTED STABLE POINT (this commit): gate-green state with proc_exit
import (fn indices +1: rt_init=3, prelude 4..61, _start=62), exc globals
section, combined name blob, FRAME_LINE null-head guard. The lowering
arms (Item::ClassDef / Stmt::Try / Stmt::Raise / Expr::Attribute /
Line->FRAME_LINE pokes / user-fn frame push-pop) were implemented and
debugged to validating-wasm EXCEPT the _start top-level try_table wrap,
which hits a residual wasmparser 'operators remaining' rejection — that
work is parked in /tmp/lower_wip.diff (40KB) and NOT in this commit;
without the wrap the modules stay gate-green and fail-closed on
try/raise (Unsupported), never miscompiling.

DEFERRED (03.03 remainder, in order):
1. Re-land the lowering arms from the parked diff; crack the try_table
   wrap validation (likely a label/type subtlety in the Catch::One shape
   vs the $catch block result).
2. EH-wrap ON by default + exceptions_wasm/classes_wasm corpus families
   with CPython pins (controlflow_wasm pattern) + ap-conform --filter
   slices byte-identical.
3. Descriptor split (data/non-data), property/staticmethod/classmethod
   dispatch, multi-base C3 merge, metaclass hooks (S-03-1/2).
4. Multi-frame traceback + source echo + 3.11+ caret anchors (needs
   per-frame source spans); tb_render is single-frame v1.
5. __init__ arity >3 (static cascade bound) -> needs a dynamic RO-array
   constructor (wasm GC has none; options: slice switch extension or an
   aprt host helper).

## 2026-11-16: leaf 03.04 session (generators/comprehensions/match — INFRASTRUCTURE LANDED, corpus gates NOT green)

STARTING STATE: HEAD 13344585 WIP snapshot (prelude_gen.rs stub + types.rs
$generator mutability). FIRST FINDING (commit 09e5f89b): the parked
/tmp/lower_wip.diff rescued in d6176427 is STRUCTURALLY BROKEN — 3-way
apply produced an unbalanced EH-wrap Block/TryTable shape + a brace
mismerge that moved code.function inside a dropped block (fnsec/code
length mismatch, "operators remaining" on every module). REVERTED to the
HEAD gate-green lower.rs; the diff file stays in git history (d6176427)
for reference only — do NOT re-apply it. Also unblocked the host: disk
was 100% full (errno=28 link failures); cargo clean on merged leaf
worktrees (leaf-0206/0209a/0209b/0302b/0205e) reclaimed 39GB.

CORPUS COMMITTED (5303a051): generators_wasm GE001-005 + match_wasm
MA001 with CPython 3.14.7 pins (stdout/stderr/exit triples; pins recorded
by /opt/homebrew/bin/python3.14 per the corpus rules). Corpus only grows.

DRIVER PRELUDE LANDED (d0f79315, ALL VALIDATING, CF 18/18 intact):
emit_gen_prelude registers 7 drivers at FN 62..68 (gen_send, iter_of,
iter_next, iter_contains, stop_iter, gen_throw, gen_close);
FN_FIRST_USER 62->69; FTs 99..109 (GEN_STEP/GEN_NEW/GEN_SEND/GEN_THROW/
GEN_CLOSE/ITER_OF/ITER_NEXT/LIST_FROM_ITER/TUPLE_FROM_ITER/ITER_CONTAINS/
STOP_ITER) declared in append_class_exc_fts. Container iterators reuse
$generator (fn=null, state=len, locals=snapshot, value=$int cursor).

WasmGC validator lessons (each cost a bisect cycle — recording ALL):
1. add() locals pairs START at the first PARAM-FREE slot: (o,o)->o fns
   start locals at 2, (o)->o and (o)->() at 1. Mismatches surface as
   "unknown local N: out of bounds" or silent wrong-type LocalSets.
2. If(Empty) arms must leave []: calls returning values need Drop even
   when they throw at runtime (the validator checks the fall-through).
3. Array refs are NOT $pyobj: array-stash locals need dedicated
   (ref null $obj_arr[_ro]) declarations — oref cannot hold them.
4. Refinements are lost through locals (35f2ace0 restated): EVERY
   struct.get/struct.set on a stashed oref needs cast-on-stack
   (lget_gen helper added for $generator).
5. call_indirect needs the i32 table idx pushed LAST (on top of params).
6. $exn-field writes need the value cast to (ref null $exception) on the
   stack — param oref is NOT the field type.
7. wasmparser renders EVERY concrete heap type as "(ref null $type)" in
   error text — identical strings, different indices (extends the
   0302b note); error text alone cannot disambiguate, dump operands.
8. stop_iter builds args=[value] straight-line; the empty-flag input is
   kept in the FT for 03.05 but ignored (the If(Result(RO)) empty-vs-
   value shape failed validation unlocatably within the timebox).

HONEST STATE vs the leaf exit gate: NOT MET. What is NOT done:
a. The LOWERING arms (generator defs -> FT_GEN_STEP state machines;
   yield/yield-from emission; comprehensions; match; For-over-iterable)
   are NOT started — scan still rejects them fail-closed. The prelude
   drivers have no callers yet.
b. iter_of body is STUBBED passthrough (commit d0f79315) — the full body
   (snapshot copy + cursor box + $generator build) was drafted but its
   local layout must be RE-DERIVED under lesson 1 before landing.
c. GE001-GE004 + MA001 gates: NOT run through ap-host (they will fail
   Unsupported at scan until (a) lands). GE005 likewise.
d. The 03.03 EH boundary (top-level try_table wrap) remains unwired —
   gen.throw/close paths need it for byte-exact uncaught rendering.
NEXT (03.04b): re-land iter_of per lesson 1; then lowering arms in
order: yield state-machine -> for-over-generator -> comprehensions ->
match (control-flow only, no EH) -> throw/close wiring with 03.03b's
EH wrap. Each with its corpus gate before the next.


## 2026-11-16: leaf 03.03b session (leaf-0303b worktree — EH wrap landed)

TRY_TABLE WRAP CRACKED + ON BY DEFAULT (61db4510, d2247196). The parked
"operators remaining" rejection is gone; modules validate through
wasmtime with the top-level try_table wrap emitted unconditionally.
Stable shape (eh_probe-proven): Block $catch(result exc-ref) {
  try_table(catch TAG_PYEXC -> label 0 = $catch) body end;
  RefNull(exc); } payload rides to $catch end; null = success, else
tb_render + proc_exit(1). CPython-format traceback verified: raise 5 ->
"TypeError: None" rc 1 (exc_raise validates BaseException-derivedness,
builds a fresh TypeError via StrTab msg + exc_new; exc-typed scratch
local keeps the throw-tag payload type). emit_try_handlers rewritten to
the same shape ($out { $catch { try_table; RefNull } isnull? Br $out :
handler chain; unmatched rethrow }) — try/except ValueError catches and
binds (e prints "None": args-message path parked).

MRO/ISINSTANCE ROOT FIXED (61db4510 + d2247196): (a) rt_init gave every
builtin type mro=[self, object] (was empty outside BaseException/object
-> isinstance(5,int) False); (b) the EXC_TREE mro "patch" was a single
ArraySet clobbering mro[0] with base.mro[0] — replaced with an inline
copy loop in rt_init's free local 0 (new prelude fns would shift
FN_FIRST_USER; ids never move); (c) the copy loop's guard had its GeU
operands swapped (len>=i instead of i>=len) so it never ran, leaving
NULL mro tails -> mro_dict_get traps + issubclass(False) — fixed.
issubclass(ValueError, BaseException)=True, isinstance(5,int)=True now.

PY_STR TUPLE REPR UN-PARKED (d2247196): the parked "values remaining"
wedge root = items local was (ref null $pyobj) but $tuple.items is an
ARRAY ref (not a $pyobj subtype) — local retyped (ref null $obj_arr_ro)
+ lget_cast at the ArrayGet. Join loop un-stubbed with CPython shape
(sep between items, 1-tuple trailing comma); two pre-existing inverted
separator conditions fixed (Eqz+Eqz = i!=0). print((1,2,'x')) ->
"(1, 2, x)" (item repr-quoting parked), e.args -> "(boom, )".

GATES: controlflow_wasm_gate 18/18 GREEN with the wrap on (regression
baseline held); eh_probe 2/2. Corpus families NOT green: 36 cases = 24
UNSUPPORTED (pre-existing: attr-assign targets "self.x = v", call arg
kinds beyond positional, 03.02c Expr slice) + 12 MISMATCH/HOST-ERR.
Biggest single unlock for family-green: attr-assign target lowering
(self.x = v in __init__), then class __init__ dispatch through user fns
(fn63+ traps), then repr item quoting + e str() message path.

Tooling notes locked: wasmtime validation offsets are BOGUS (point into
the data section) — attribute failing fns via
wasm_encoder::tests/diag_wtvalidate (wasmtime::Module::new names the
fn); wasmparser validates some things wasmtime rejects, so wasmparser
pass != pass. GeU/Ge_s operand order bugs (a>=b vs b>=a) produce silent
wrong-branch bugs — check operand order FIRST when a wasm loop guards
wrong.
