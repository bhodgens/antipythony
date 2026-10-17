# Bughunt 2026-10-17 - parent pre-read findings (verified at HEAD 4bf9f77)

Wave anchor: 4f3d033..4bf9f77 (all 38 commits). 10 auditors dispatched, one per commit batch.
Parent ran baseline gates on a clean detached worktree /tmp/ap-headcheck.

## Baseline verdict (clean HEAD worktree)

- cargo test --workspace: FAILED. 24 suites ok, 1 suite red:
  ap-run/tests/stdlib_modules_differential.rs -> 19/19 tests panic
  `spawn ap-run-eval: Os { code: 2, kind: NotFound }`.
- pytest tests/: 4 failed, 1265 passed, 165 skipped, 594 xfailed, 90 xpassed.
  Failures: T109-exception-in-thread, s005_io_streams, s016_collections_abc_isinstance,
  s023_ast_parse_dump_unparse (all test_expected_bytes_stable).
- After `cargo build --release` in the worktree, the 19 stdlib differential tests PASS
  (TEST_EXIT=0). Root cause is binary-path resolution, not behavior.

## P1 HIGH - stdlib_modules_differential hardcodes target/release/ap-run-eval

File: crates/ap-run/tests/stdlib_modules_differential.rs:33-37
Evidence: `concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/release/ap-run-eval")`
`cargo test` builds DEBUG. On any tree without a prior `cargo build --release`,
all 19 tests panic NotFound. The main repo passes only because target/release/
holds a stale binary from an earlier manual release build. A fresh clone or CI
run is red; worse, the test silently compares against a STALE release binary
when one exists (it can pass against code that is not HEAD).
Verified: true (baseline panic + green after release build).
Fix shape: move the test to crates/ap-cli/tests/ (the package that owns the
ap-run-eval bin) and use env!("CARGO_BIN_EXE_ap-run-eval"), which cargo builds
automatically and points at the right profile.

## P2 HIGH - 4 corpus pins embed the absolute checkout path

Files (expected stderr pins containing /Users/caimlas/git/antipythany/...):
- tests/corpus/fstrings/expected/f33_bytes_adjacent_error.err
- tests/corpus/stdlib/expected/s016_collections_abc_isinstance.err
- tests/corpus/stdlib/expected/s023_ast_parse_dump_unparse.err
- tests/corpus/threading/expected/T109-exception-in-thread.stderr
The suites run the case by ABSOLUTE path, so CPython tracebacks print that path.
Pins captured in the original checkout therefore fail at any other path
(worktrees, CI, another machine). Verified: true (4 pytest failures in
/tmp/ap-headcheck, byte diff at the path prefix).
Fix shape: run corpus cases with a path relative to REPO_ROOT (cwd=REPO_ROOT
already set) so tracebacks print a stable relative path, then regenerate the
4 affected pins from live 3.14.7. No normalization inside the comparator.

## P3 HIGH - s005 golden is locale-dependent; f0a4c22's correction claim is false

File: tests/corpus/stdlib/expected/s005_io_streams.out line 3 = `utf-8 utf-8`.
Commit f0a4c22 message claims: "reference prints 'utf-8 utf-8' under all local
locales". FALSE. CPython io.text_encoding(None) returns the literal string
'locale' outside UTF-8 mode; it returns 'utf-8' only under UTF-8 mode.
Verified: true - `python3.14 -c "import io,sys; print(io.text_encoding(None),
sys.flags.utf8_mode)"` prints `locale 0` in the ambient env; PYTHONUTF8=1
prints `utf-8`. The suite does not pin the locale environment, though
docs/compat-contract.md:499 says comparisons hold "under the pinned locale
environment" - the pinning step is missing.
Fix shape: pin the corpus-runner environment (PYTHONUTF8=1 or LC_ALL=C.UTF-8)
in the T-11/T-10/T-05/T-06 runners, keep the utf-8-mode golden, and re-verify
all pins under the pinned env.

## P4 MEDIUM - ap-conform reference-interpreter fallback has no version check

File: crates/ap-conform/src/lib.rs:84-91
Evidence: `for candidate in ["python3.14", "python3"] { if which_exists(...) }`
which_exists (lib.rs:93-100) only runs `--version` and checks spawn success;
it never checks the version STRING. A machine without python3.14 silently uses
any python3 (3.9, 3.12...) as the "CPython 3.14 reference", making every
differential verdict meaningless while still printing green.
Verified: true (ran with PATH stripped of python3.14; harness reported
"reference interpreter: python3" and compared normally).
Fix shape: parse `--version` output, require major.minor == 3.14, else Error
outcome (fail closed).

## P5 LOW - ap-conform declares unused dependency shell-words

File: crates/ap-conform/Cargo.toml (`shell-words = "1"`); zero uses in src/.
Verified: true (grep). Harmless; remove or wire in.

## Refuted suspicions

- ap-conform CAN fail: negative control (`--impl /bin/echo`) produced
  FAIL + exit 1. The harness is not a green-always gate.
- compare_case compares stdout, stderr, and exit code byte-exactly; no
  normalization/masking in the comparator. Timeout and spawn-failure paths
  both produce non-Pass outcomes (never a silent pass).
- expand_corpus exits 2 when the glob matches zero files (no 0/0 green).

## Disclosure ledger (open at dispatch time)

- Working tree carries a SIBLING's uncommitted WIP: crates/ap-run/src/
  {builtins,classes,exc_hier,frame,interp,lib,methods}.rs modified,
  crates/ap-run/src/importer.rs + vendored_stdlib/ untracked. Files last
  written 11:04-11:06, minutes before this session started. Another agent
  owns 02.04a. This wave does NOT touch those files; auditor 10 reviews them
  read-only for disclosure.
- 90 xpassed in baseline not yet triaged (xfail(strict=False) suites).
- pytest baseline ran in /tmp/ap-headcheck (moved path) - that is what
  exposed P2; in the main checkout those 4 pass.
