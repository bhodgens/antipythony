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

## 05.02 IMPLEMENTED NOT COMMITTED — ap-parse-check seam bin (2026-10-20)

### What landed
- NEW `crates/ap-parse/src/bin/ap_parse_check.rs` + `[[bin]]` entry in
  `crates/ap-parse/Cargo.toml`. clap style per 05.01 (name+version, positional
  `<file.py>`). No other files touched; zero test edits.

### Contract derivation (from the two T-01 suites' code, not the brief)
Both suites resolve the same bin name/env (`AP_PARSE_CHECK` -> PATH ->
target/debug -> target/release) but pin DIFFERENT reject renderings of the
same `ap_parse::ParseError`:
- `tests/t01_parse`: reject line 1 `^error at line L, column C: <msg>$`
  (== ParseError's existing byte-exact Display), rc==1 EXACT, stdout empty,
  accept stdout byte-exact `OK <n> statements` (no trailing newline, pinned
  by test_trivial_single_statement_ok_and_count), missing file rc==2.
- `tests/t01_parse_diagnostics`: any stderr line matching
  `^(path):(<line>):(<col>)` with the fixture basename + reference position.
Rendered as TWO stderr lines (Display shape then file:line:col shape over the
identical line/col/message fields). Neither regex matches the other's line,
so both "exactly ONE diagnostic" fail-fast counts hold (G-7 safe). Message
text is ap-parse's existing ruff-derived formatting (same source
ap-verify-dump's remap prints for parse failures); prose is free per C-10.
Columns = ap_parse::line_col_of (Unicode scalars, the G-2 character reading).

### Gate results
1. `cargo build -p ap-parse` — OK (0.1.0 bins all built).
2. `pytest tests/t01_parse_diagnostics tests/t01_parse -q` (baseline 105
   xfail in t01_parse_diagnostics + 33 xfail in t01_parse = 138 seam tests)
   -> `50 xfailed, 88 xpassed, 0 failed`. All 88 xpass are in
   tests/t01_parse_diagnostics (17 genuinely-failing bodies stay absorbed as
   xfail by strict=False; enumerated below). t01_parse stays 33 xfail — see
   FINDING-3, its bodies cannot run real regardless of the bin.
3. XPASS delta for the ceiling conversation: +88 xpassed, ALL from
   `tests/t01_parse_diagnostics/test_t01_parse_diagnostics.py` (suites:
   test_valid_program_accepted x30, test_syntax_family x14,
   test_invalid_program_rejected x11, test_diagnostic_has_file_line_col x11,
   test_diagnostic_position_matches_reference x7,
   test_first_error_only_fail_fast x1, test_no_import_tokens x14).
   tests/XPASS_CEILING=74 (owner: 01.01/01.02) is breached on the next full
   run unless the markers are burned/flipped in a follow-up — reported, not
   edited here (ceiling file + test markers are outside this leaf's
   ownership).
4. rustfmt --edition 2024 --check on the new bin: clean.
   `cargo clippy -p ap-parse --all-targets`: exit 0, zero warnings attributable
   to ap_parse_check.rs.
5. Spot green: t02_ir_shape + t04_ir_verify -> 65 passed / 3 skipped;
   t03_lowering_semantics + corpus_traceability -> 477 passed / 1 skipped /
   41 xfailed (unchanged).

### FINDINGS (real behavioral gaps between pinned expectations and
### ap-parse's existing parse surface — NOT bin bugs; per xfail policy these
### stay red/xfail, fixed by impl or contract PR, never by test edits)
F-1 ACCEPT-vs-REJECT class (ruff parser is syntax-only):
    break_outside_loop, nonlocal_module (module level), star_import_in_function
    parse CLEAN (rc 0) — CPython raises them at compile/symtable stage. The
    placement rules live in ap-ir verify (01.04 surface), not in parse().
    11 t01_parse_diagnostics cases stay xfail from this class: 3 fixtures x
    {invalid_rejected, file_line_col, position} = 9, plus focused
    test_star_import_in_function_s04_10 + test_nonlocal_module_level_s02_3.
F-2 position-anchor drift (ruff offsets != CPython 3.14 offsets):
    unclosed_paren impl (4,1) vs pinned (2,5); unicode_unclosed_paren impl
    (4,1) vs pinned (2,5); py2_print impl col 7 vs pinned (1,1) (t01_parse
    pins the OTHER reading, col 7 — the two suites disagree, both xfail
    sides documented in T-01 doc G1/G2); invalid_fstring impl (2,14) vs
    pinned (2,13). 7 position cases + 3 focused (py2_print/star/unicode) +
    t01_parse equivalents stay red-in-xfail.
F-3 tests/t01_parse harness bug (FINDING, not touched): its `ap_parse_check`
    session fixture returns a bare `PosixPath` when the bin exists, but every
    body calls `runner.run(...)` (only `_MissingBin` has `.run()`). Verified:
    33/33 --runxfail failures are `AttributeError: 'PosixPath' object has no
    attribute 'run'` — zero cases ever exercise the bin. Emulating the
    fixture's INTENT against the built bin: 28/33 pass; the same F-1/F-2
    impl gaps account for the other 5 (break_outside_loop,
    nonlocal_module_level, star_import_in_function accepted; unclosed_paren +
    unicode col drift). Needs a one-line test-infra fix by the owning leaf —
    out of 05.02's zero-test-edit scope.
G- wiring note: ci.yml fast-lints does not yet invoke the t01 suites with the
    bin path; the resolver finds target/debug/ap-parse-check without env when
    the build step precedes, so wiring is a ci.yml change owned by the
    parent's 05.02 gate follow-up (this leaf's ownership excluded .github/).
Not committed, no git add, per dispatch. Sibling dirty files untouched.

## 05.03 IMPLEMENTED NOT COMMITTED (2026-10-20, leaf agent)

### Lint policy (root Cargo.toml)
- `[workspace.lints.rust]` RESERVED block replaced: rustdoc tool-lints do NOT
  belong there - added a NEW `[workspace.lints.rustdoc]` table with
  `broken_intra_doc_links = "warn"` (cargo 1.94 accepts the `rustdoc` lints-tool
  table; verified `cargo metadata` parses). Every crate with `[lints]
  workspace = true` now warns on broken links at normal build time.
- Escalation to deny stays in RUSTDOCFLAGS on the CI doc step: rustdoc lints
  set through Cargo `[lints]` cannot be promoted to errors by RUSTFLAGS, and
  the `-D warnings` env is the single escalation point (also catches
  non-link rustdoc warnings like private_intra_doc_links / ambiguous links).

### CI step text (ci.yml NOT edited per dispatch - orchestrator wires)
Add to the rust job, after build:

```yaml
      - name: cargo doc (rustdoc gate, 05.03)
        run: RUSTDOCFLAGS='-D warnings' cargo doc --no-deps -p ap-obj -p ap-ir -p ap-parse -p ap-run -p ap-conform

      - name: cli-docs drift gate (05.04)
        run: |
          cargo build -p ap-cli -p ap-parse -p ap-conform --bins
          cargo run -p xtask -- cli-docs --check
```

### Gate result (locally, 2026-10-20)
- `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps -p ap-obj -p ap-ir -p
  ap-parse -p ap-conform` -> EXIT 0. (ap-obj/ap-parse had real warnings;
  fixed below.)
- WITH `-p ap-run` -> EXIT 101: 14 warnings in ap-run + ap-gc. ALL 14 sit in
  files carrying sibling 02.07 threading work-in-progress (builtins.rs,
  classes.rs, exc_hier.rs, gcsupport.rs, importer.rs, methods.rs, sre.rs,
  threading_call.rs) - dispatch ownership says NEVER other crates, so NOT
  fixed here. Pre-existing at HEAD for the stable subset (verified via
  `git show HEAD:...` greps for each warning line). BREAKING THE ap-run
  DOC-GATE TALLY for the orchestrator: fix = wrap Python-index brackets in
  backticks (args[0] -> `args[0]`, [object], obj[key], [script_dir],
  vars([obj]), [IDENT]) + `<module>` -> `&lt;module&gt;` or backticks in
  importer.rs:803. Suggested follow-up leaf 05.03b after 02.07 lands; until
  then run the CI step WITHOUT -p ap-run (4-crate scope, exit 0 proven) or
  accept red.
- ap-obj fixed: 4 broken links (classes.rs [object], int.rs [BigInt],
  value.rs items[i]/items[lo..hi] - all Python-subscript text mistaken for
  links; backticked) + 1 unused-import warning (ops.rs PyDict moved into
  #[cfg(test)] mod tests where it is only used).
- ap-parse fixed: lib.rs `[ap_parse::ParseError]` self-crate path (unresolvable
  in own crate docs) -> `[ParseError]`; verify_bridge.rs `[ap_ir::verify]`
  ambiguous fn-vs-mod -> `[ap_ir::verify()]`; private-item link
  `[attach_positions]` -> de-linked to `attach_positions`.
- ap-ir/ap-conform: zero rustdoc warnings already.

### missing_docs policy (review roadmap item 2: 3 public surfaces only)
- Probe: `RUSTDOCFLAGS='-W missing_docs' cargo doc --no-deps -p <crate>` count.
- ap-conform: 0 undocumented pubs -> `#![warn(missing_docs)]` ADDED to
  crates/ap-conform/src/lib.rs (zero-warning gate met).
- ap-ir: 0 undocumented pubs -> `#![warn(missing_docs)]` ADDED to
  crates/ap-ir/src/lib.rs (zero-warning gate met; re-doc job still exit 0).
- ap-obj: 56 undocumented pubs -> NOT added (per dispatch: honesty over
  ambition). Follow-up leaf: doc the 56 surface items then add the warn.
  ap-parse/ap-run stay internals-doc-free per roadmap item 2.

### Gates executed (05.03)
1. cargo metadata OK with [workspace.lints.rustdoc]
2. RUSTDOCFLAGS='-D warnings' cargo doc 4-crate scope: exit 0 (post-fix)
3. cargo test -p ap-obj -p ap-ir -p ap-parse -p ap-conform: all green
4. rustfmt --check clean on all touched files; clippy -p xtask + the 4 crates
   introduces no new warning classes (doc-list-indent warnings pre-existing
   at HEAD, not introduced by doc-line edits)

## 05.04 IMPLEMENTED NOT COMMITTED (2026-10-20, leaf agent)

### Design
`crates/xtask/src/cli_docs.rs` (new) + dispatch in main.rs + `workspace_root()`
made pub(crate). SHELL-OUT design per dispatch: runs each built bin's
`--help` + `--version` from target/debug (honors CARGO_TARGET_DIR), zero bin
refactors. CLIS table (order = render order): ap-run-eval, ap-obj-eval
(ap-cli); ap-ir-dump, ap-verify-dump, ap-parse-check (ap-parse); ap-conform
(ap-conform). Missing/tolerated bin -> "NOT BUILT ... regenerate" placeholder
section (ap-parse-check tolerance for concurrent 05.02; at this run time
ap-parse-check HAD landed and its real help is rendered - 0 placeholders).
Output is path-independent (no absolute roots in the file) so the CI
regeneration diffs cleanly.

### PROVE
- `cargo run -p xtask -- cli-docs` -> wrote docs/testability-clis.md,
  6 CLI sections, all real clap help text.
- `cargo run -p xtask -- cli-docs --check` -> exit 0 (self-consistent).
- Injected "drift" line -> --check exit 1 with drift message -> restored ->
  exit 0. Drift gate proven both directions.
- `cargo test -p xtask` -> 18 unit (incl. 3 new cli_docs tests: pure-render
  determinism + missing-bin tolerance, CLIS membership, collect arity) +
  4 integration gates, all green.
- Note: sibling sa-1 mid-edit had a transient syntax error in
  crates/xtask/src/stdlib_provenance.rs (line 765, escaped-bracket corruption);
  cleared by the time tests ran - NOT my file, not my fix; main.rs edits were
  re-read + merged over their stdlib-provenance dispatch (kept theirs intact).

### CI step text
Included in the 05.03 block above (cli-docs drift gate step).

### Gaps / notes
- ci.yml untouched per dispatch (both step texts here for orchestrator).
- ap-run excluded from the proven doc-gate scope (sibling dirty files); see
  05.03b tally. The CI YAML above includes -p ap-run as the orchestrator's
  final target state; expect it red until 02.07's files get the bracket fixes.
- No commits, no git add, per dispatch.
