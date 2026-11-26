# PROGRESS — 00-infra/02-frame-and-entry (fast-path leaf 02)

Branch: tier1-fp-frame @ 0b3fc97 (worktree ~/.git/worktrees/leaf-fp-frame),
cut from main @ e92c829. Three commits:
1. c8f5257 — BytecodeInterp + line bookkeeping + --tier bytecode toggle
2. 1fe0e76 — fix_bytecode.py gate + unbound-locals fix (Vec<Option<Value>>)
3. 0b3fc97 — StoreSubscript operand-order fix + gate probes that actually
   bytecode-route

## What landed

- `crates/ap-run/src/bytecode_interp.rs` (927 lines): the dispatch loop.
  LOCATION DECISION (spec allows either crate): ap-run, because every
  opcode routes through the tree-walker's shared helpers (eval_binop /
  compare_values_pub / eval_unary_pub / lookup_name_pub / call_value /
  subscript_get+set / attr_get_public) and ap-bytecode is a pure
  translation layer (adding ap-run as its dep would invert the layering).
  - explicit pc over Vec<Instr>; fixed slot-indexed locals
    Vec<Option<Value>> (None = unbound; Some(Value::None) = real store);
    operand stack Vec<Value>. No recursion for straight-line code;
    CallFunction recurses via call_value (the tree-walker's path).
  - LINE BOOKKEEPING: thread-local Cell<u32> (BC_LINE) written per
    instruction; frame.last_line/line_track synced ONLY at instrument ops
    (BinOp/UnaryOp/Compare/Subscript/LoadAttr/CallFunction — the ops whose
    helpers dispatch user dunders / can emit warnings) and at the error
    exit (exception construction → frame-exit traceback read). Dense
    per-instr line vec expanded once from the sparse LineTable at
    cache-insert.
  - compile cache keyed by def-Arc ptr (same discipline as jit_hook's
    JIT_CACHE); fallback decided once per function. Generators + bodies
    containing AugAssign refuse to compile (aug needs the __iX__ in-place
    protocol, not BinOp+store) → tree-walker fallback.
- `eval_unary_pub` extracted from the walker's UnaryOp arm (interp.rs);
  the walker now calls the same fn — single source of semantics.
- Entry toggle: TierMode::Bytecode (`--tier bytecode` / APX_TIER=
  bytecode), ap-cli pre-scan + cmd_run forward + ap_run_eval + ap-conform
  --tier accept it. DEFAULT OFF = dispatch arm never entered
  (bit-identical to main; jit_enabled modes untouched). [BC-STATS]
  compiles/fallbacks/execs under APX_TIER_STATS.
- `tests/bench/primitive/fix_bytecode.py` (643 lines): the both-walkers
  gate. 8 probes (arith / strings / comparisons / scoping+shadowing+
  closures / traceback-linenos / displays / unbound / deliberate-
  fallback), three legs (tree-walker, APX_TIER=bytecode, CPython 3.14),
  stdout+stderr+rc byte-identical, PLUS an exercise assertion
  (APX_TIER_STATS on the bc leg must show execs>0 — a silently-
  tree-walking toggle cannot pass).

## Bugs the gate caught (both fixed)

1. LoadFast used Value::None as the unbound sentinel → `x = None; x`
   raised UnboundLocalError. Locals are now Vec<Option<Value>>.
2. StoreSubscript popped v/k/o and called subscript_set(o,k,v), but the
   lowerer's layout is [value, obj, key] (Assign pushes value FIRST) —
   args scrambled; `d.x[k] = v` raised "'int' object does not support
   item assignment" (s015 failed under the toggle). The earlier
   subscript probes passed only because `{}` displays fall back
   wholesale — the gate now builds dicts via dict() so the bytecode
   StoreSubscript path is genuinely exercised.

## Gate results (final tree @ 0b3fc97, release build, fresh target-fp-frame)

- fix_bytecode.py: 8/8 GREEN (three-way byte-identical, exercise>0).
- ap-conform values 50/50, stdlib 48/48 — toggle ON and OFF.
- fix_int / fix_json / fix_sort / fix_random / diff_b64 / diff_hex:
  PASS with toggle ON and OFF. fix_sha_zlib: tw==bc byte-identical;
  29/818 corrupt-stream zlib.error text divergences vs CPython are the
  DOCUMENTED zlib-rs message-granularity residual (fix-sha-zlib report;
  identical both toggles — pre-existing, not a bytecode regression).
- s016/s016b/s016c line probes (e016..e020): GREEN both toggles.
- cargo test -p ap-bytecode -p ap-run: 242 passed / 0 failed.
- fp smoke: three-way identical vs CPython, toggle on AND off.

## Honest gaps / filed (NOT this leaf's regressions; verified tw==bc)

1. Read of a later-ASSIGNED local before its assignment: tree-walker
   raises plain NameError, CPython UnboundLocalError (compile-time local
   classification — lowerer already slot-assigns, but the walker's
   runtime read path is the divergent side).
2. Qualname of a method defined in a class nested in a function
   (`attr_boom.<locals>.R.__radd__` vs CPython's `__radd__`); linenos
   match exactly.
3. Full traceback.print_exc TEXT: CPython 3.14 caret annotation lines
   (pre-existing walker divergence; bytecode identical to walker).
4. zlib corrupt-stream error texts (zlib-rs granularity) — documented.
5. AugAssign bodies fall back (needs __iX__ opcode — 01-core).
6. Out of scope per spec: control-flow execution (For/While/Try bodies
   still fall back), caches, superinstructions.

## Perf note (not gated this leaf)

No perf claim is made (leaf 03-rollout owns the bench). The dispatch
loop is correct-but-conservative: every semantic goes through the same
helpers the walker calls; the walk-floor kill (opcode dispatch without
AST re-visits + TLS-only line bookkeeping) is the mechanism 02-perf
optimizes from here.
