# Branch 05 - CLI + Docs: PROGRESS

## 05.01 IMPLEMENTED NOT COMMITTED (2026-10-20, leaf agent)

Migrated the 4 hand-rolled testability bins to clap:

- `crates/ap-cli/src/bin/ap_run_eval.rs` - `--tier <MODE>` + positional `<FILE>`
- `crates/ap-cli/src/bin/ap_obj_eval.rs` - trailing_var_arg expression tokens (empty -> stdin)
- `crates/ap-parse/src/bin/ap_ir_dump.rs` - `--tree`/`--golden`/`--verify` ArgGroup (mutually exclusive) + positional `<FILE>`
- `crates/ap-parse/src/bin/ap_verify_dump.rs` - positional `<FILE>` only
- `crates/ap-cli/Cargo.toml` + `crates/ap-parse/Cargo.toml`: added
  `clap = { version = "4", features = ["derive"] }` - EXACT string mirrored from
  `crates/ap-conform/Cargo.toml:16` (per-crate version string; there is no
  `[workspace.dependencies]` clap key).
- All 4 bins gained `-h/--help` + `-V/--version` (workspace version 0.1.0
  confirmed via `bin --version` -> `ap-* 0.1.0`). `about` comes from the bin's
  own rustdoc line, NOT the crate description (dropped `about` from
  `#[command]` so clap renders the struct doc).

### Byte-identical proof battery (26 cases, script /tmp/battery_0501.py)

Pre: hand-rolled bins built at HEAD-equivalent state -> `/tmp/battery_pre/`.
Post: clap bins -> `/tmp/battery_post/`. `diff -r` shows ONLY the 6 bad-arg
usage-error cases differ (see below); the other 20 (all valid invocations +
runtime errors) are byte-identical, and the full RC list is identical
(`/tmp/pre_rc.txt` vs `/tmp/post_rc.txt`: RC_IDENTICAL).

Battery cases:
- ap-run-eval: values corpus V001 (bare / --tier 0 / --tier=0), syntax-error
  file, missing file, no-args, --tier adaptive, --tier (no value), two positionals
- ap-obj-eval: `1 + 2`, multiword tokens, stdin mode, syntax-error expr
- ap-ir-dump: default, --tree, --golden, --verify (clean probe + flagged probe
  rc3), syntax-error, missing file, no-args, --bogus flag
- ap-verify-dump: silent probe, flagged probe, missing file, no-args

### bad-arg handling (the 6 diffs - INTENTIONAL, contract-safe)

Grep over all of `tests/` (incl. t01/t02/t04/t05/t06/t07/t11 + conftest/env
overrides): NO suite pins stderr bytes or rc for a BAD-ARG invocation of any of
these bins. Suites only pin: valid-invocation stdout/stderr/rc (t02 asserts
rc==0 then diffs stdout; t04 parses stdout lines + rc 0/1 on VALID probe
paths; t05/t06/t07/t11 pass only `<file>` or `--tier 0 <file>` through
APX_IMPL/ap-conform). The 6 changed cases are pure usage errors:
- `eval_noargs`, `irdump_noargs`, `vd_noargs`: `usage: ap-X ...` -> clap
  "the following required arguments were not provided" (rc 2 preserved)
- `eval_tier_novalue`: `--tier requires a mode` -> clap "a value is required"
  (rc 2 preserved)
- `eval_two_pos`, `irdump_badflag`: hand-rolled messages -> clap
  "unexpected argument ... found" (rc 2 preserved)
Clap's arg-error rc (2) matches the hand-rolled rc (2) in every case.
`ap-verify-dump --help` previously printed the usage line with rc 2; now
prints clap help with rc 0 - no suite exercised that path (verified by grep).

### Gates executed
1. `cargo build -p ap-cli -p ap-parse` - OK (bin warnings only from sibling
   dirty ap-run, pre-existing)
2. Battery diff - 20/26 byte-identical; 6 diffs are ONLY the bad-arg cases
   above; RC list identical
3. `pytest tests/t02_ir_shape tests/t04_ir_verify tests/t01_parse
   tests/t01_parse_diagnostics -q` -> 65 passed / 3 skipped / 138 xfailed
   (t01 suites drive the missing `ap-parse-check` bin via AP_PARSE_CHECK ->
   xfail-by-design, leaf 05.02's job; t02/t04 drive ap-ir-dump/ap-verify-dump
   real runs). t03_lowering_semantics: 1 passed / 41 xfailed (pins
   `--scopes`, seam not implemented - unchanged by this leaf).
   APX_IMPL=target/debug/ap-run-eval: t05+t06+t07 -> 499 passed / 13 xfail /
   64 xpass; t11_stdlib -> 284 passed / 2 skip / 79 xfail / 63 xpass
   (xpass rates pre-existing, no golden edited)
4. `rustfmt --edition 2024 --check` on the 4 bin files - clean (repo-wide
   `cargo fmt --check` diffs ONLY sibling-dirty crates/ap-run/src files)
5. `cargo clippy -p ap-cli -p ap-parse --all-targets` - exit 0, zero warnings
   on the 4 bins

### Gaps / notes
- ap-conform `--impl` forwarding appends `--tier 0 <file>` - verified clap
  accepts that arg order (battery eval_v001_tier0 byte-identical).
- `ap-obj-eval --help` now exits 0 with clap help (was: joined into expr
  stream); trailing_var_arg keeps `-1 + -2`-style hyphen tokens positional.
- irdump form flags are mutually exclusive (ArgGroup) - hand-rolled parser
  silently preferred args[0]; no suite ever passes two form flags (grepped).
- Not committed, no git add, per dispatch. Sibling dirty files untouched.
