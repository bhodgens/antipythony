# Leaf S016C session record — 2026-11-22 (worktree ~/.git/worktrees/leaf-s016c, branch tier1-s016c off main@3cc80a6)

## Status: DONE — the S016 campaign's three probed-but-not-built instrument
## classes (UnaryOp / IfExp / Attribute) extended per the preserved
## PROBES-S016B.md recipes, probe-first, 2 new pinned corpus cases, all
## runnable gates green. Session ended early by user direction (a gate
## command hit the permission layer; wrapped up lean per instruction).

## Probe-first findings (CPython 3.14.7, /opt/homebrew/bin/python3.14)
Ground truth re-measured this session (tb_lineno walks + warnings
stacklevel=2 fired directly inside the dunders, per the S016B recipe —
first attempt routed warn() through a helper and measured nothing; the
stacklevel must cross exactly one user frame):
- **UnaryOp**: `-` alone on line A, operand on A+1 -> warns from
  `__neg__` at the OPERATOR line A (== node start), not operand or
  statement line. Single-line `-n` inside an earlier-starting display
  also node-start. DIVERGENT -> EXTENDED. Operand CALL keeps the call
  line (probe u2; s016 contract holds).
- **IfExp**: o4 opener `(1 <nl> if Boom() <nl> else 2)` — node starts
  A, cond on A+1 -> attributes A+1 = CONDITION line. o5 (`1 if (
  Boom2() )`) cond line too. DIVERGENT (the one shape S016B left open)
  -> EXTENDED.
- **Attribute**: q1 obj / dot / attr on 3 lines -> attr-NAME line; q2
  obj / `.attr` glued -> attr line; q3 backslash-continuation -> attr
  line. NOT obj start. DIVERGENT -> EXTENDED. ruff Attribute node
  start == obj start, so the stamp comes from `a.attr.range().start()`.

## Fix (commit 6919d1c, 12 files; corpus 1706d4a)
Exact s016b pattern, one trailing `usize` per variant (0 = unknown):
- ap-ir `Expr::UnaryOp(UnaryOp, Box, usize)` /
  `Expr::Attribute(Box, Name, usize)` / `Expr::IfExp(Box, Box, Box,
  usize)`. All walk/verify/dump/builder pattern sites updated; Builder
  constructors keep 0; hand-built-IR test constructions updated with
  `, 0` (verify.rs test, cfg_tests, feedback_tests, tierup_tests).
- ap-parse lowering stamps: UnaryOp `line_at(u.start())` (operator ==
  node start); Attribute `line_at(a.attr.range().start())` (the
  semantic contract — NOT node start); IfExp `line_at(i.test.start())`.
  `Target::Attribute` reconstruction keeps 0 (store path, never an
  instrument site).
- ap-run interp.rs: publishes to `frame.last_line` AFTER operand/obj/
  cond evaluation, BEFORE the dunder-slot dispatch (line > 0 guard
  preserved) — unary dunders, __getattr__ dispatch, __bool__ dispatch
  all read the instrument line.
- JIT (cfg.rs) / Wasm (lower.rs) pattern sites updated, fields ignored
  (verify_bridge confirms IR shape superset).

## Differential probe (pre-corpus, both sides byte-identical)
8-shape probe (u1/u2/u3/o4/o5/q1/q2/q3): stdout+rc identical
ap-run-probe release vs CPython 3.14.7. Probe file removed before
commit (evidence in this record).

## Corpus (append-only; 2 new cases, pins from CPython 3.14.7)
- exceptions/e019_unaryop_instrument_lines.py — raising __neg__ with
  the operator alone on its own line (tb walk) + warnings stacklevel=2
  same shape + operand-CALL-keeps-call-line control.
- exceptions/e020_attribute_instrument_lines.py — raising __getattr__
  with obj/dot/attr on 3 lines + `.attr`-glued continuation + same-line
  control. KeyError-based, NOT AttributeError: a user `__getattr__`
  raising AttributeError has a PRE-EXISTING envelope divergence
  (message text `'AB' object has no attribute...` vs the raised
  message, and the tb walk drops the __getattr__ frame) — verified
  identical on a plain same-line probe, NOT introduced by s016c,
  disclosed in the case header and not pinned. tb frame co_name
  qualname shape (`Cls.__getattr__`) is the documented 02.03 gap;
  pins are linenos only.
- Both: ap-run-probe tier-0 output byte-identical vs CPython 3.14.7
  (stdout+stderr+rc). .meta via tests/tools/_gen_meta.py; the 18
  pre-existing metas' date-only churn reverted; only the 2 new metas
  kept. FUZZ_SEEDS.txt foreign append (21 lines) appeared in the
  worktree and was reverted (not mine to commit; s016b precedent).

## Gates (measured this session, this worktree, this tree)
- ap-conform stdlib slice (the s016 gate) --tier 0: **48/48**
- ap-conform values slice --tier 0: **48/48**
- ap-conform exceptions slice --tier 0: **20/20** (18 old + e019 + e020)
- ap-ir + ap-parse unit suites: all green (84 tests total, 0 failed)
- ap-run full suite (release, CARGO_TARGET_DIR exported): **206 passed /
  0 failed / 1 ignored** — matches the s016b baseline count exactly
  (incl. classes_differential 29/29, method_surface 34/34, gc 42/42).
  GOTCHA re-confirmed: first run 2 fix_pass FAILED = spawn ENOENT until
  the DEBUG bins (ap-run-eval) are built with the same CARGO_TARGET_DIR.
- ap-codegen-wasm scan_vet_contract: 1/1 ok
- ap-run suite re-check: 18/18 suites `test result: ok`
- **Permission-blocked, NOT re-run (user direction)**: the combined
  `cargo test --release -p ap-codegen-native -p ap-codegen-wasm` run
  was interrupted (one invocation killed at the 300s kernel timeout,
  a re-run blocked by the permission layer, a third timed out at the
  420s terminal cap). No result is claimed for the full codegen-native
  / codegen-wasm suites THIS session beyond scan_vet_contract; the
  compile-check over both crates passed earlier (`cargo check --tests`
  green) and the only codegen test edits are 3 hand-built-IR `, 0`
  additions of the same shape s016b landed, so risk is low but the
  suites are UNMEASURED this session — filed as the follow-up
  verification step. controlflow_wasm differential and the
  classes/gc/threading baseline-envelope slices likewise NOT run this
  session (time/permission envelope; no code path they exercise
  changed — interp.rs arms publish an extra line only when the IR
  carries line > 0, i.e. only for the newly stamped shapes).

## Honest gaps / follow-ups
1. Full ap-codegen-native + ap-codegen-wasm test suites: UNMEASURED
   this session (permission-blocked mid-run, disclosed above). Re-run
   `cargo test --release -p ap-codegen-native -p ap-codegen-wasm` with
   CARGO_TARGET_DIR set before merging tier1-s016c.
2. controlflow_wasm differential + classes/gc/threading baseline
   envelope slices not re-run this session (same envelope).
3. Compare chains: CPython has per-chain-link instruments; we stamp
   the chain's first-operand line (inherited from s016b, unchanged).
4. adaptive-tier s023 red (pre-existing, ast.unparse surface).
5. qualname shape `Cls.__dunder__` vs `__dunder__` in tb frame names
   (documented 02.03 envelope; corpus pins avoid depending on it).
6. User `__getattr__` raising AttributeError: message + missing
   __getattr__ tb frame (pre-existing envelope, discovered en route
   while pinning e020; disclosed, not pinned, not fixed here).
