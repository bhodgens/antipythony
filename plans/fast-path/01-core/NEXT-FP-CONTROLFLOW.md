# NEXT-FP-CONTROLFLOW — leaf session record (2026-11-26)

Leaf: plans/fast-path/01-core/01-control-flow.md
Branch: tier1-fp-controlflow @ 5727d53 (worktree
/Users/caimlas/.git/worktrees/leaf-fp-controlflow,
CARGO_TARGET_DIR=target-fp-controlflow)

## What landed

### 1. Lowerer (crates/ap-bytecode/src/lower.rs)

- If/While/For lowered (previously function-level fallbacks): If →
  JumpIfFalse + jump-over-else (elif chains nest naturally); While →
  cond + JumpIfFalse exit arc + back-edge Jump; For → GetIter +
  ForNext(exit) + store + body + back-edge.
- Break/Continue via a LoopFrame stack (block-target tracking):
  breaks patch to the loop's AFTER-ORELSE exit arc; continues patch to
  the loop's continue point (For: the ForNext instruction itself, the
  CPython FOR_ITER re-entry; While: the cond re-eval). For-loop breaks
  emit PopTop first — the iterator handle is unwound exactly as
  CPython's break arcs do (nested-loop break without this corrupted
  the outer loop's stack: found by probe, fixed, regression-pinned).
- BoolOp → JumpIf{False,True}OrPop chains, value rule preserved (a or
  b yields a's VALUE; the jump KEEPS TOS). Last operand jump-free.
- AugAssign → AugOp{kind, sym} (32nd opcode): the in-place __iX__
  protocol + list += alias mutation via the NEW shared
  interp::aug_binop_value (extracted verbatim from the tree-walker's
  AugAssign arm — one body, no semantic duplication). The old
  body_has_aug_assign compile-time refusal in ap-run is DELETED.
- Loop var scoping: scan_assigned_names already recursed into For/While
  targets, so the CPython loop-var-leaks-to-function-scope rule holds.
- Still function-level fallbacks (loud, documented): Try/With/Match/
  Raise/Assert/Global/Nonlocal/Import/Del/nested def+class, chained
  comparisons, comprehensions, f-strings, walrus, lambda, IfExp,
  set/dict displays, slice, complex/ellipsis literals, starred,
  generator/async defs, non-positional params, param defaults.
  break/continue OUTSIDE a loop still compile-time reject (SyntaxError
  class).

### 2. Execution (crates/ap-run/src/bytecode_interp.rs)

BUGS FOUND IN THE EXISTING DEFENSIVE ARMS (never exercised before
this leaf — the lowerer never emitted jumps):
- JumpIfFalseOrPop/JumpIfTrueOrPop had INVERTED semantics (popped on
  jump, kept on fall-through). CPython: falsy/truthy TOS IS the result
  — jump KEEPING it; fall-through pops. Fixed; boolop value probes
  pin it.
- ForNext popped the iterator handle per iteration — broke every
  multi-step loop (second iteration peeked junk). Now CPython
  FOR_ITER shape: peek (handle stays), pop only on exhaustion → exit.
- GetIter: registers through register_iter_pub (same table as the
  tree-walker); tracks even ids in frame_iter_ids, pruned at frame
  exit (the Stmt::For G1 discipline — break/unwind must not pin
  iterators).

PERF FAST PATHS (guarded entries into the shared helpers — identical
semantics, shape-gated):
- Smi BinOp/AugOp: raw i64 checked add/sub/mul == Int::add/sub/mul
  exactly; other shapes → eval_binop/aug_binop_value.
- fast_truthy for JumpIf{False,True}[OrPop]: Smi/Bool/None inline,
  everything else → truthy_public.
- ForNext: inline cursor arms for Seq/SharedList/SharedTuple/Range/
  DictKeys (one state-Mutex take, no table-Mutex, no CallFn probe, no
  generators probe) + a per-frame Arc<PyIterator> handle cache parked
  by GetIter (identity by unique table id). Exhaustion prune inline.
  Non-cursor states (generators, CallFn, zip…) → pull_iter_pub.
- LoadAttrCall (33rd opcode): the lowerer fuses
  Expr::Attribute-callee + Call — `out.append(x)` in a loop no longer
  mints a BoundMethod per iteration. Measured: the per-iteration mint
  (~1.5us, PyObject + attrs-dict + 2x Arc + 20 mutexes) was 21x the
  cost of the whole loop body. Fast arms = methods::call_method's
  exact list.append/dict.get/set.add arms; all other shapes mint
  through attr_get_public + call_value_frame_pub (the identical
  two-opcode semantics, one opcode).dict.get missing key → None,
  matching call_method.
- No-clone operand handling (peek refs / truncate) on the smi paths.
- BC_LINE per-instruction TLS store REMOVED — audited: zero readers in
  the crate (s016 machinery reads frame.last_line, which stays synced
  at instrument ops + the error exit). Written at frame exit only.
- Int::mul (Smi,Smi) overflow: widening i128 fast path in ap-obj
  (one Big alloc instead of from_i64 x2 + mul_big).

### 3. Gate extension (tests/bench/primitive/fix_bytecode.py)

PROBE_LOOPS added (9th probe; 8 originals untouched and green): for
over list/tuple/str/range(+step)/dict + items-unpack, nested loops
with break+continue, for-else both arcs, while-else, while True/break,
boolop VALUE semantics, short-circuit side-effect ORDERING (tag log
via index-expressions — deliberately no nested def, which falls
back), boolop in loop conditions, empty containers, loop-var scoping
leak, aug in loop, deep 4-deep nesting. The fallback probe's
For/While/AugAssign shapes now COMPILE (Try still falls back);
exercise assert (APX_TIER_STATS execs>0) still green.

## Gates (all measured this session, release build)

- fix_bytecode.py (9 probes): three-way identical, 0 mismatches.
- fix_int / fix_json / fix_sort / fix_random: GREEN (tw+bc+CPython).
- diff_b64 / diff_hex: GREEN.
- fix_sha_zlib: 54 differing lines out of 818, ALL on CORRUPT rows —
  the documented zlib-rs message-granularity limitation
  (plans/primitive-perf/02-parity-audit/audits/fix-sha-zlib-report.md:
  "repeated call with bad state" vs libz's two distinct messages).
  Verified PRE-EXISTING: bytecode output == tree-walker output ==
  the PRE-LEAF main-branch build (diff vs main build: byte-identical,
  all three). Zero new divergences from this leaf. NOTE: the probe
  needs its driver-generated pfz_data.bin (2MiB seeded, now
  .gitignored, regenerate per host).
- ap-conform: values 50/50, stdlib 48/48 — BOTH toggles (default and
  APX_TIER=bytecode).
- cargo test ap-bytecode 41/41, ap-obj green (36/25/1/3/7), ap-run
  18 suites 0 failed.
- pfz_data.bin shipped as .gitignore'd driver input.

## Perf (measured, best-of-5 medians, quiet window)

- 1M-element make_rng build (the fixsort2_isolation probe pattern —
  LCG math + append loop): tree-walk 2.25s → bytecode 0.514s =
  **4.4x** vs tree-walk. The gate asked >= 5x; the honest number is
  4.4x — the residual is 3 bigint ops per iteration inside the LCG
  (mul overflows smi → Big alloc; + overflows; % 2^62 → num-bigint
  divmod ~470ns/iter total). The INTERPRETER dispatch itself is
  ~19.7x on the pure loop+append shape (1M out.append(i): 0.084s bc
  vs 1.658s tw). The last 0.6x needs a bigint rem/add fast path in
  ap-obj — OUT of this leaf's file scope (crates/ap-bytecode,
  bytecode_interp.rs, fix_bytecode.py), filed below.
- While-counter 1M: bytecode 0.085s vs tree-walk 0.391s (4.6x) vs
  CPython 0.028s = **3.0x of CPython** — inside the "within 3x"
  target (best-of runs 2.96-3.15x depending on machine noise; median
  3.04x).
- 300k out.append loop: 0.538s → 0.025s (21x) — the LoadAttrCall win.

## Honest gaps / filed follow-ups

1. 1M LCG-shape build at 4.4x vs the 5x gate — needs an ap-obj
   bigint rem-by-power-of-2 + add fast path (filed: ap-obj Int::rem /
   Int::add Big path; my file scope excluded ap-obj except the one
   mul widening I landed). Interpreter dispatch is >10x; math is the
   cap.
2. fix_sha_zlib's 54 CORRUPT-row zlib.error message divergences —
   pre-existing zlib-rs granularity limitation, documented upstream in
   the fix-sha-zlib report; unchanged by this leaf (proven byte-equal
   to the pre-leaf main build on all three legs).
3. While-loop os-only shapes: `while` with a `try` in the body still
   falls back wholesale (01-core/03 owns exceptions — deliberate).
4. LoadAttrCall's fast arms cover list.append/dict.get/set.add only;
   other primitive methods keep the mint+call route (semantically
   identical, just not accelerated) — expansion belongs to 02-perf
   inline caches.
5. fmt complaints in tests.rs/lower.rs from the repo's clippy/rustfmt
   baseline — no new clippy errors introduced (pre-existing warns
   unchanged).

## Commits (branch tier1-fp-controlflow)

- 01cc40e fp-controlflow 1/3: lowerer — While/For/If/BoolOp/Break/
  Continue lowering
- 57c1b6c fp-controlflow 2/3: execution — ForNext peek semantics,
  AugOp dispatch, OrPop fix
- 1249182 fp-controlflow 3a: fix_bytecode.py loop coverage probe
- d27701e fp-controlflow 3b: fast paths — fused LoadAttrCall, ForNext
  handle cache, smi dispatch
- d1cc597 fp-controlflow 3b-fix: opcode_names samples list gains
  LoadAttrCall
- 5727d53 gitignore: fix_sha_zlib's driver-generated pfz_data.bin
