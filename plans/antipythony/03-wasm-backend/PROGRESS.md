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


## 2026-11-16: leaf 03.03c session (leaf-0303c worktree — classes/exceptions lowering landed)

COMMITS f16c3932, 57f46a76, 935850d5 (branch tier1-0303c off 472ec565).

LANDED (all byte-exact vs CPython 3.14.7 through ap-host):
1. attr-assign targets: self.x = v lowers via FN_ATTR_SET (scan interns
   the attr name; emit stashes value → obj → name). The parked-diff
   replacement (structural per 03.04's finding) was REIMPLEMENTED fresh —
   the parked diff was NOT applied.
2. ROOT-CAUSE FIXES that unblocked class __init__ dispatch (the fn63+
   HOST-ERRs): (a) attr_set stored the OBJECT as the value (LocalGet(3)
   → LocalSet(5); value is param 2) — every class-body bind wrote the
   class itself, so the later __init__ RefCast($function) trap-failed;
   (b) Stmt::LocalDef's class-body binding pushed [fn, cls, name] but
   attr_set takes (obj, name, v) — validation failed; (c) user-class
   Name-calls route through FN_CALL_OF → cls_call (Scan.classes); the
   $function cast trapped on $type objects.
3. Class-body plain assigns (k = 42) bind via attr_set through
   TEMP_LOCAL_B (A holds the class stash; clobbering A passed the value
   as obj). scan interns Name-assign targets.
4. py_str: str_from_bytes stamps the str TYPE global (null type field
   made re-dispatch fall through — str(1) printed "None"); tuple join
   quotes str items (repr semantics); 1-tuple tail comma without space
   (('v',) byte-exact).
5. try/else (S-02-32): orelse on the success path only — handler chain
   wrapped in one extra Block; success Br lands on the orelse, handler
   completion Brs exit the whole try.
6. scan fail-closed rejects for super/NotImplemented name reads (were
   emit-time panics); nested class names interned (class-in-try swept
   clean — corpus_wasm_sweep no longer panics).

GATES: classes_exceptions_wasm_gate 16/16 (NEW corpus families
classes_wasm CLW001-008 + exceptions_wasm EXW001-008, controlflow_wasm
pattern, CPython 3.14.7 pins); controlflow_wasm_gate 18/18 held;
ap-codegen-wasm all suites 0 failed; ap-host 3/3.

HONEST STATE vs the leaf title: attr-assign ✓, class __init__ dispatch ✓
(0..=3 static arity as built), repr item quoting ✓ (print path; no
quote/backslash ESCAPING inside items — deferred), multi-raise: raise e
re-raise ✓, raise from still HOST-ERRs (FN_EXC_CTX_SET path), bare
raise inside except still HOST-ERRs (G_ACTIVE_EXC not maintained by
handlers). super cases: fail-closed UNSUPPORTED (documented, honest).
e str() message: FN_EXC_MSG wired into py_str via an excinst arm —
arm is IN but shadowed by the NoneType arm at runtime: the caught exc's
observed type-sel is 8 (NoneType) — root cause NOT cracked in-session;
print(e) prints "None" (deferred, top of the 03.03d list).
NOTE: corpus_wasm_sweep (classes/exceptions native families) remains
diagnostic-only — those families cover the FULL S-03 surface (decorators,
descriptors, slots, metaclass) far beyond the wasm slice; 0 of 48 pass
there and that is expected/honest for this leaf.

Tool notes: wasmtime trap Display lacks the reason — format {e:?} for
"wasm trap: <reason>". cargo test test-binaries CAN cache stale content
but the EXAMPLE binary rebuilt fine each time; the real trap this
session was COMMITTING A STALE prelude.rs (57f46a76 lacks its claimed
fixes — a later full-file write overwrote them; the fixes re-landed in
935850d5). Lesson: re-read the diff BEFORE committing after
full-file-write edit cycles.

NEXT (03.03d): e-str sel==8 root cause (check exc_raise payload type
field / rt_init EXC_TREE aliasing — print(ValueError-instance) hits
sel==8); bare-raise + raise-from (G_ACTIVE_EXC maintenance in
emit_try_handlers); __init__ arity >3; multi-base C3; descriptor split.

## 2026-11-16: leaf 03.04b session (drivers real + py_str surfaces; corpus gates NOT green)

COMMITS: merge 01fcdbd1 (main 520f4abe into tier1-0304b — the stale afee87ac WIP was a
pre-main-state snapshot; merged, NOT reset), then d15b3a4f increment 1.

LANDED (d15b3a4f, all validating, CF 18/18 + classes/exceptions 16/16 held):
1. iter_of FULL BODY (was stub passthrough): $list/$tuple snapshot copy -> $generator
   {fn:null, state:len, locals, value:$int(0), exn:null}; $str -> 1-char-per-byte
   snapshot (latin1 slice); $generator identity; other shapes TypeError
   ("<type> object is not iterable" via exc_new+exc_raise). LESSON-1 RE-DERIVED: the
   parked iter_of draft assumed params 0/1 — FT_ITER_OF has ONE param so param-free
   locals start at 1 (the snapshot helpers now use 3=snapshot 4=RO items 5=byte_arr
   6/7/8=i32).
2. stop_iter REAL THROW (was stub): exc_new(StopIteration, [value]) + exc_raise.
   args stash rides a DEDICATED (ref null $obj_arr_ro) local — lesson 3 cost another
   bisect: array refs are NOT $pyobj and storing (ref 16) into an oref local fails
   validation as "expected (ref null $type), found (ref $type)" (both render identically).
3. gen_throw: state<=0 (not-started OR dead) raises the thrown exc at the call site
   without running the body (3.14 arm; GE003 pre-start throw).
4. gen_close: state==0 parks CLOSED without running the body (3.14 close-before-start).
5. py_str: list arm "[" + repr-join + "]" and dict arm "{k: v}" — str keys AND values
   quote-wrapped (CPython dict repr). Dict entries fetched STACK-RESIDENT via two
   array.gets (an $entry_arr-local + sget path was tried; $dict_entry is NOT a $pyobj
   subtype so no local stash is possible — new lesson 9: only $pyobj-subtypes may ride
   oref locals).
6. attr_get: StopIteration.value = args[0] (e.value, GE002 plumbing); empty args -> None.

TOOLING (worked well, record for next session): the "identical error text" ambiguity
(lesson 7) is best cracked by wasmprinter Config.print with a Print impl that overrides
start_line(binary_offset) — prefixes every wat line with @offset; wasmparser's error
offset then maps to the EXACT wat line. Cycle: validate -> @offset dump -> read the
failing instr + its operand types -> fix the Rust emit. Three type-index bugs found this
way in ~3 cycles each.

HONEST STATE vs the leaf exit gate: NOT MET. GE001-GE005 + MA001 all still fail-closed
at scan ("generator/async def (leaf 03.04)", "Stmt::For", "Stmt::Match", comp/genexp
Expr arm). The drivers have no lowering callers yet for user generators; iter_of/iter_next
serve only as the For/comp targets once those land.

NEXT (03.04c — design fully derived this session, see below):
a. GENERATOR LOWERING = CFG FLATTENING (not nested resume labels: wasm br only goes
   outward, so a resume into a While body requires flat slices). Shape:
     block $done { loop $head {
       block $v0 { ... block $vN { br_table [state i -> depth i] default -> $done }
       } <slice N-1> } <slice N-2> } ... <slice 0> } }   ;; slice i inside $v(N-i-1)
   Every slice: reload ALL slot mirrors from $gen.$locals, run its straight-line code,
   end `br $head`. Slot mirrors = wasm oref locals 2..2+S (FT_GEN_STEP has TWO params;
   param-free locals start at 2). Slot table = params then collect_local_binds order
   (same filter as emit_user_fn). st scratch local after the mirrors.
   Yield K: spill all slots; $gen.$state = K+1; $gen.$value = yieldval; Frame-safe:
   generators must NOT push tb frames (or push/pop around each resume — decide in 03.04c);
   return yieldval. Resume path: slice K reloads, and the yield-expression's value =
   $gen.$value (parked by gen_send BEFORE the step — input rides $value).
   return v: spill; state=GS_DONE; $value=v; return null. Fall-off-end: state=GS_DONE;
   $value=null.
   yield from e: it = FN_ITER_OF(e); loop { try_table-catch -> $yf_done { x =
   FN_ITER_NEXT(it); <yield x as a normal site> } ; handler: r = attr_get(e_caught,
   "value"); br loop-exit } — the Stmt::Try lowering shape works INSIDE the step body.
   b. Def site: emit_function_object unchanged (the $code carries func_idx). CALL sites:
   emit_call + dispatch/static paths special-case generator slots (fn.def is_generator):
   build $generator{type, fn=$function, state=0, locals=array of bound args, value=null,
   exn=null} instead of calling. gen_send's table idx math (func_idx - FN_FIRST_USER)
   works because generator step fns ARE user-fn table entries (element segment has all
   slots) — but their FT in the function SECTION must be FT_GEN_STEP (gen o, input o)->o.
   c. FOR statement: iter = FN_ITER_OF(e) once; loop { try x=FN_ITER_NEXT(it) catch SI ->
   br exit; bind target; body } ; else on normal exit. Unpacking targets stay fail-closed.
   d. BUILTINS: next(x[,d]) = try FN_ITER_NEXT catch SI -> default; range(n)/list(x)/
   sum(x) = drain via next into a growable list (rebuild-append inline — the dset rebuild
   pattern; no new prelude fn) / sum accumulates via FN_BINOP ADD.
   e. COMPREHENSIONS: inline loop lowering in an own-scope slice; clause 0 iter evaluated
   in the ENCLOSING scope; nested clauses in comp scope; walrus binds ENCLOSING (S-02-2).
   ListComp acc = growable $list rebuild-append; DictComp acc = mut $entry_arr
   rebuild-append then StructNew(T_DICT) with used=fill=n; SetComp fail-closed this leaf.
   Genexp = hidden generator fn with one yield inside the clause loops (reuses (a)).
   f. MATCH (MA001 slice): per case: Literal -> FN_EQ with the lit; Capture -> bind;
   Wildcard -> match; Or -> try each with binding rollback (bindings ride locals so
   "rollback" = only bind on final success: evaluate alternates into scratch then bind);
   Sequence [p...] on $list/$tuple with len checks + Starred rest (growable list);
   Mapping {k: v} on $dict via FN_DGET (str-key slice) + FN_DICT_HAS pre-check (missing
   key = no match); Guard after bindings; Class/Value patterns fail-closed (not in MA001).
   g. GE002 specifics: c.send(v) = FN_GEN_SEND; first-send-None = send(None) surface;
   StopIteration as e: print('SI value', e.value) needs `except StopIteration as e` +
   e.value (attr arm LANDED in this increment) + FN_EXC_MSG for str(e).
   h. GE003: e = ValueError('mid') + raise e INSIDE the generator body — the try lowering
   inside the step body catches it via try_table ✓ (exceptions inside generators are just
   EH inside the step fn; gen_send propagates the tag out to the resumer's handlers).
GATES: controlflow_wasm_gate 18/18 held; classes_exceptions_wasm_gate 16/16 held;
GE/MA status verified fail-closed via a throw-probe (report line in session log).
