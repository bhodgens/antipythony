# Leaf S016 session record — 2026-11-20 (worktree ~/.git/worktrees/leaf-s016, branch tier1-s016 off main@cb41e88)

## Status: DONE — s016 GREEN, stdlib slice 48/48 tier-0. The "staged-source
## degradation" label was wrong: this was never the staged-source path — it
## was statement-granularity line tracking colliding with CPython's
## per-instrument (per-bytecode) line table.

## Root cause (3-layer dissection, per the brief)
1. PARSE (ruff 0.0.4): NOT the culprit. ruff carries full byte spans on
   every Expr node incl. multi-line Call/List displays — positions were
   always available. No upstream ruff fix needed; dep NOT upgraded.
2. ap-parse LOWER: the real loss point. The IR is span-free BY DESIGN
   (master contract: "vendor-clean ap-ir"); `Lowerer::lower_expr`'s
   `ast::Expr::Call` arm dropped the ruff range entirely.
3. ap-run RUNTIME: consumed statement-granularity line info only.
   `frame.last_line` (which feeds `f_lineno` via `py_frame_stub`, and
   hence warnings attribution + tracebacks) was only ever written by
   `Stmt::Line` markers (lowering emits one per statement start). A
   multi-line list display therefore pinned EVERY expression inside it to
   the display's FIRST line (s016: `checks = [...]` starts line 14; the
   `isinstance(b"s", ca.ByteString)` call sits on line 21). CPython's line
   table is per-instrument: the CALL bytecode carries its own line, so
   warn()'s stacklevel walk reports 21. Warning TEXT already matched
   (02.08f's observation); only the LINE diverged.

## Fix (commit ef99fab, 13 files, +67/-20)
- ap-ir `Expr::Call(Box<Expr>, Vec<Arg>, usize)`: third field = the call's
  1-based source line (0 = unknown). All walk/verify/dump pattern sites +
  the 2 hand-built-IR test constructions updated (line 0). Builder
  `call()` keeps 0; the Lowerer stamps after construction.
- ap-parse `lowering.rs`: Call arm stamps `line_at(c.start())` (ruff Call
  node start == callee start for every direct form; probed: multi-line
  callee/parenthesized forms attribute identically to CPython).
- ap-run `interp.rs` `eval_expr` Call arm: `frame.last_line.set(line)`
  BEFORE evaluating args/callee (line > 0 guard) — warnings raised
  mid-display read the executing call's line, matching CPython's CALL
  bytecode line. JIT/AoT paths untouched (cfg.rs ignores the new field;
  Stmt::Line pokes unchanged — verify_bridge confirms IR shape superset).
- ap-conform `expand_corpus`: canonicalize case paths so BOTH interpreter
  sides embed the same `__file__` SHAPE in warnings. Discovered en route:
  after the line fix, s016 STILL failed on stderr byte 0 — CPython
  resolves the path it's handed (absolute here) while ap-run kept the raw
  relative glob form (`tests/corpus/...` vs `/Users/...`). Harness bug,
  not interpreter bug; canonicalize is a no-op for absolute globs.
  NOTE: pytest t11 asserts internal consistency of ap-conform, not path
  shapes; no test edits needed.

## Verification probes (beyond the gate)
Multi-line display/call shapes probed byte-identical vs CPython 3.14.7:
- multi-line list display (the s016 shape): 21 both sides
- multi-line dict display + wrapped call args: call-start line both sides
- call spanning 3 lines, callee alone on its line: line 5 both sides
- wrapped call args on a continuation line: line 6 both sides
- lambdas (single-line and body-wrapped) warning from a display: line of
  the lambda's warn call both sides
- traceback shape for a raise inside a wrapped call: CPython shows fine-
  grained carets `^^^` (SyntaxError-adjacent position columns); we show
  the standard two-line frame + source echo. This divergence is the
  DOCUMENTED v0 caret-fidelity gap (run_module SyntaxError comment),
  pre-existing, unrelated to lineno attribution.
- AoT binary of s016: stdout+stderr byte-identical vs CPython (after
  worktree-path sed) — the cfg.rs path is unaffected and correct.
- adaptive tier: stdlib slice 47/48 with s023_ast_parse_dump_unparse
  red — VERIFIED PRE-EXISTING (same 47/48 with s023 red on the un-amended
  baseline tree built in this worktree; s023 is an ast.unparse surface
  gap from the 02.08 audit list, adaptive-only manifestation, untouched).

## Gates (all measured this session, this worktree, this tree)
- ap-conform stdlib slice --tier 0: **48/48** (s016 GREEN; the target)
- ap-conform values slice: 48/48 (no regression)
- ap-conform exceptions slice: 15/15
- controlflow_wasm differential: 1/1 ok
- scan_vet_contract (wasm): 1/1 ok
- classes_exceptions_wasm_gate: 1/1 ok
- ap-ir + ap-parse unit suites: all green (incl. collect_stmt_lines
  brute-force oracle, fold, verify)
- ap-codegen-native unit suites: all green (cfg/tier1 tests updated for
  the 3-field Call)
- ap-run full suite (18 suites, CARGO_TARGET_DIR pinned): ALL GREEN —
  incl. classes_differential 29/29, method_surface 34/34,
  import_differential 29/29, gc_differential 42/42(?), fix_pass 2/2.
  (First run showed 2 fix_pass failures = spawn ENOENT: the suite resolves
  target/debug/ap-run-eval via CARGO_TARGET_DIR env; a `cargo test` without
  the env var set resolves a non-existent worktree-relative path. Build
  the debug bins + export CARGO_TARGET_DIR — machine gotcha, NOT a code
  regression. Same class as the AGENTS.md stale-bin gotcha.)
- corpus families vs CPython (tier-0), before/after compared ON THE SAME
  TREE (stash round-trip): classes 28/33→28/33, gc 19/20→19/20, threading
  17/18→17/18, tier1-transparency 16/17→16/17 — ZERO movement. The 5
  classes fails (c002 mappingproxy, c018 vars, c030/c031/c032 metaclass
  hooks) + gc018 (qualname `Outer.<locals>.Inner.<locals>.Deep`) + T109 +
  t1_007 are all in the documented 02.03/02.08 honest-gap envelope
  (PROGRESS.md: "classes 27/33 held" era counts vary with later module
  landing; the per-case IDENTITIES match the documented gaps exactly).

## Parallel-safety incident (disclosed honestly)
Mid-session, a `git stash`/`git stash pop` round-trip in this worktree
collided with SIBLING state: the pop applied a foreign stash
("On tier1-0209a: test-run side effects" — mbc_tables/mbc_gb18030_4 +
a 38MB binary blob + FUZZ_SEEDS.txt with conflict markers). Recovery:
unstaged + restored all foreign files; verified my commit dd87eff had
swept 19 FUZZ_SEEDS seed-addition lines (append-only, zero deletions vs
main) via `git add -A`; AMENDED to ef99fab restoring seeds byte-identical
to main. Final commit touches exactly my 13 files. The foreign stash
entry remains in the stash list (untouched, not mine to drop). No
crates/ap-run mbc* content landed in any of my commits (verified via
git show --name-only).

## Machine policy compliance
- ONE cargo lane; CARGO_BUILD_JOBS=4 throughout; all builds timed out at
  550s budget, no lock contention observed.
- Worktree + logs under ~/.git/worktrees/leaf-s016 (never /tmp).

## Honest gaps / follow-ups
1. The Call-line fix covers CALLS. CPython's line table is finer still
   (every instrument: BinOp/Compare/Attribute ops carry opcodes with
   positions). A traceback raised INSIDE a multi-line BinOp expression
   (non-call) would still show the statement line. No corpus case gates
   this today; if one lands, the same pattern extends (Expr gains an
   optional span or per-variant line fields). Filed here, not built.
2. adaptive-tier s023 red (pre-existing, ast.unparse surface).
3. The `git add -A` discipline trap (AGENTS.md 02.06 lesson) fired again
   via the foreign stash — consider `git add <explicit paths>` for
   parallel-agent worktrees; filed for the orchestrator.
4. ap-run suite path-resolution gotcha (CARGO_TARGET_DIR must be exported
   for suites spawning ap-run-eval) documented above; candidate for
   AGENTS.md gotcha list.
