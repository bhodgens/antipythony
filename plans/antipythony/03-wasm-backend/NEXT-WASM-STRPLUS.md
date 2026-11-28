# NEXT-WASM-STRPLUS — leaf session record

- **Worktree**: ~/.git/worktrees/leaf-wasm-strplus (branch `fix-wasm-strplus` off main@4186206;
  main@4186206 contains 468e24d — all three defects reproduced on it first-hand)
- **Date**: 2026-11-27
- **Input**: three "NEW base defects" from `.hermes/audits/2026-11-26-bughunt-addendum.md`
  §6.1; method = probe-first, RED corpus before fix, byte-exact vs
  /opt/homebrew/bin/python3.14 (3.14.7).

## Root causes

### Defect 1 — `str + str` traps in FN_BINOP
`binop` (prelude.rs) computed both type selectors (3 = str) but had NO str arm:
str operands fell into the int-coercion arms whose `ref.cast $int` traps
(function 34). Fix: two new arms ahead of the int/int arm, built from the two
selector locals exactly like the int/int `If`:
- str/str + ADD → `FN_STR_CAT` (existing (str,str)->str driver);
- str(3)/int(0) + MUL → new `FN_STR_REPEAT` driver (appended prelude fn 76;
  FT_S_REPEAT = 110; FN_FIRST_USER moved 75 -> 77).
Both arms are `If(Empty)` + `Return` — NOT `If(Result(oref()))`: an
`if (result T)` yields a phantom T after its end even when both branches are
dead (`return`/`unreachable`), corrupting the stack for the arms below (found
by wasmparser validation: "values remaining on stack at end of block").
str_repeat: byte $str over `ArrayNewDefault + ArraySet`, n<=0 -> EMPTY $str
(CPython), count capped at 0x7fff (huge-count MemoryError surface out of
slice — recorded divergence). The local-grid contract (NEXT-WASM-SUPER: 2
params -> 1 pad entry) bit twice: first draft declared too few locals
("unknown local 4"), then used raw local reads for the $str (oref local can't
feed `struct.get $str` — "expected (ref null $type), found i64"). Reads go
through `lget_cast(4, T_STR)`. str + non-str (CPython TypeError) still falls
through to the trailing Unreachable — loud trap, recorded divergence (the
TypeError raise rides the 03.05 EH slice).

### Defect 2 — nested print corrupts the outer print
Root shape: the print cursor lived in a PER-FRAME local (TEMP_LOCAL 40,
initialized PRINT_BUF by every frame prologue). `print('A', inner())`:
outer appended 'A ' to the buffer; inner's frame re-initialized its own
local 40 to PRINT_BUF, its append CLOBBERED the outer's partial bytes, its
flush re-emitted them, and the outer then appended at its stale old cursor —
'inB'. Cursor save/restore alone is INSUFFICIENT: the inner flush would still
re-emit the outer's partial bytes because both operated on [PRINT_BUF..end).
Fix (global cursor + disjoint-range flush):
- new GLOBALS G_PRINT_END (cursor) and G_PRINT_DEPTH (nesting depth), emitted
  after G_PENDING_EXC (print_end_g = pending_exc+1, print_depth_g =
  pending_exc+2);
- print emission (BOTH emit_builtin_call and gen_emit_builtin): save cursor
  into the PRINT_SAVES memory stack (new const 33920, 16 i32 slots indexed by
  depth), bump depth, append args via the global cursor, then
  `print_flush(start, end)` on the DISJOINT range, restore cursor, decrement
  depth;
- print_flush keeps the classic fd_write call shape (iov@8, n@16) but with
  {buf: start, len: end-start+1} instead of {PRINT_BUF, ...}.
TWO harder bugs found en route:
1. The start CANNOT ride any local (CMP/A/B/S scratch — nested arg emission
   clobbers all of them); it must ride the memory stack. First attempt with
   TEMP_LOCAL_CMP produced 'A inneA B'.
2. `i32.store` pops [addr, value] — the save stored operands SWAPPED
   (cursor 33920 into address buffer; slot stayed 0): the flush read start=0
   and re-emitted raw memory 0..16386 (16386 bytes of stdout garbage). The
   address goes first; probe tooling (hand-built wasm + ap_host) pinned it.
Correctness bar met: nested print in an argument (CLW015), in a method called
from an argument (CLW016), in a generator driven from an argument (CLW017) —
all byte-exact vs CPython. Deep nesting composes because each print occupies
its own slot while live.

### Defect 3 — `print(A.m)` traps
`print(<function>)` fell through py_str's dispatch to the null-$str return;
print_one's `RefCastNullable $str` on null traps (function 16). Decision
(recorded as designed divergence): deterministic no-address reprs
`<function NAME>` — for defs, lambdas (`<function <lambda>>`), class attrs
(`A.m` -> `<function m>`), and bound methods (`a.m` -> `<function m>`, the
$code name). CPython's shapes carry runtime addresses
(`<function f at 0x...>`, `<bound method A.m of <...>>`,
`<built-in function len>`); byte-exact pins are impossible AND un-pinnable a
second way: subscript/slicing is UNSUPPORTED on the wasm emitter
("subscript read (post-03 emitter surface)"), so `str(a.m)[:17]`-style pin
bleeding cannot be used at pin time. Deterministic shapes are the only
stable pin target; CPython parity deferred until the emitter grows slicing
or a repr-address scheme. Implemented in py_str ahead of the null fallback:
RefTest $function -> "<function " + $code.name + ">" via FN_STR_CAT (acc
local 6, tmp local 8 — local 7 is the ROARR stash, NOT a $str; first draft
used 7 and would have mis-typed).

## Baselines (main@4186206, before fix)
- CLW013 (str+str): host error fn 34 / cast $int.
- CLW014 (str*int): host error fn 34.
- CLW015/016/017 (nested print): 'inB' / 'meM' / 'geG' vs 'A B' / 'A M' / 'A G'.
- CLW018 (function repr): host error fn 16.
- d2d mixed print also exposed `print(1, 2.5, True, 's')` -> `2` (float repr
  truncates: py_str float arm I64TruncSatF64S -> int_to_str; pre-existing,
  documented gap in py_str's float arm comment — NOT this leaf's region,
  recorded below as a filed finding).

## Results after
- RED corpus: tests/corpus/classes_wasm/ CLW013..CLW018 (+ expected/
  .out/.err/.rc/.meta; metas recorded by tests/tools/_gen_meta.py under
  CPython 3.14.7; CLW018 pins are OUR documented deterministic shapes, NOT
  CPython bytes — stated in the .py header).
- Commits (fix-wasm-strplus):
  - 12c6aaf fix(wasm): str+str / str*int binop arms + deterministic function
    reprs — prelude.rs/emitter_state.rs/types.rs AND the lower.rs
    print-scheme plumbing (globals G_PRINT_END/G_PRINT_DEPTH, PRINT_SAVES
    save-stack, print arms, print_flush disjoint range). The concern split
    between code commits is imperfect: the print scheme shared plumbing
    files with the binop/repr work and rode the first commit; 12c6aaf's
    message covers binop + repr; this record documents the actual layout.
  - 47740ac fix(wasm): nested-print corruption via shared global cursor +
    disjoint flush — full message + the RED corpus pins CLW013..CLW018.
  - b251afd docs(wasm): this record.
- Gates (first-hand, CARGO_TARGET_DIR=target-leaf-wasm-strplus):
  - classes_exceptions_wasm_gate: 1 passed / 0 failed (corpus 27 passed /
    0 failed incl. the 6 new)
  - controlflow_wasm_gate: 3 passed / 0 failed
  - corpus_wasm_sweep: 1 passed / 0 failed
  - cli_corpus_dump: 1 passed / 0 failed
  - ap-codegen-wasm --lib: 1 passed / 0 failed
- rustfmt --edition 2021 --check on all five touched files: clean.
- '+' diff lines containing 'unsafe': 0 (ratchet grep clean).

## Honest scope / divergences recorded
1. str + non-str: CPython raises TypeError; wasm traps loud (Unreachable).
   The raise needs the 03.05 EH surface. Divergence noted in corpus .py.
2. str * non-int: same treatment (trap, not TypeError).
3. str_repeat count capped at 0x7fff; huge-count MemoryError surface out of
   slice. Wide $str16/$str32 repetition NOT implemented (latin1 slice only).
4. Function reprs are deterministic no-address shapes (see defect 3) —
   deliberate divergence from CPython's address-bearing shapes.
5. NOT FIXED (out of region, filed): float repr truncation
   (`print(2.5)` -> `2`; py_str float arm, pre-existing on main). This leaf
   owns py_str, but the float repr fix is a repr-formatting project (CPython
   repr(2.5)='2.5', shortest-roundtrip) too large for this slice; corpus
   pins avoid floats today. Filed for a repr-formatting leaf.
6. NOT verified: str + str / str * int inside GENERATOR step frames beyond
   the corpus shapes (binop is frame-independent — same prelude fn — so
   risk is low), and >16-deep print nesting (PRINT_SAVES has 16 slots;
   corpus nesting is <=3).

## Process notes
- The local-grid contract (NEXT-WASM-SUPER) and the operand order of
  wasm_encoder stores are the two recurring traps; probe-first with
  wasmparser::Validator + wasmprinter (offset-annotated) found every
  validation failure with exact offsets.
- Hand-built module + ap_host::run (fwtest scratch, removed before commit)
  was the fastest way to pin the save-store operand-order bug: byte-level
  stdout dumps of the iov region identified the swapped [addr, value].
- The i32.store operand-order trap generalizes: ANY new memory store in a
  prelude fn should assert its operand order against the wat dump.
