# NEXT-D13 — leaf D13 session record (2026-11-19)

Closes 04.03's D1 + D3. Branch `tier1-d13`, worktree
`~/.git/worktrees/leaf-d13`, off main 5b09003.

## What landed (commit c2b171e)

### D3 — installed-binary `build` self-sufficiency (THE fix)

- `crates/ap-cli/src/build.rs`: the CRT staticlib lookup now probes
  `$PREFIX/lib/antipythony/libap_aot_crt.a` FIRST, where
  `$PREFIX = dirname(dirname(exe_path))` — the scripts/install.sh
  layout — then falls back through the existing chain
  (CARGO_TARGET_DIR, the exe's own target dir, checkout
  target-0206/ + target/ dirs). An INSTALLED binary (04.03 release
  archive) no longer fails the build verb loudly on a fresh machine.
- Refactored through a `crt_staticlib_from(exe_path, target)` seam so
  unit tests can point exe resolution at a staged prefix layout; the
  production `crt_staticlib` feeds `current_exe()`.
- The not-found error now names `$PREFIX/lib/antipythony/...` +
  scripts/install.sh so a fresh-machine failure is actionable.
- Unit tests (in build.rs `#[cfg(test)]`):
  - `crt_lookup_finds_staged_install_prefix_crt` — stages a real
    `<prefix>/bin/antipythony` + `<prefix>/lib/antipythony/
    libap_aot_crt.a` layout in a temp dir, asserts the lookup resolves
    the staged CRT, and the negative arm asserts the missing-CRT error
    names the install path.
  - `install_prefix_crt_none_for_bare_path` — degenerate-path no-panic.

### D1 — `--version` + `--notices`

- New `crates/ap-cli/src/notices.rs` module (separate functions +
  separate file to stay clear of the 04.05 agent's main.rs verb lanes);
  main.rs touched ONLY in the run() dispatch match + usage string.
- `antipythony --version` (also `version`, `-V`): prints name +
  `CARGO_PKG_VERSION` + short commit when `AP_COMMIT_SHA` was set at
  build time (option_env!; release.yml may set it per-build — plain
  cargo builds print version only, honest, not invented).
- `antipythony --notices` (also `notices`): prints the EMBEDDED
  THIRD_PARTY_NOTICES byte-exact to stdout (include_bytes! from the
  repo-root file, raw write_all — no lossy String round-trip).
- DECISION RECORDED: embedding is build-time from the repo-root
  THIRD_PARTY_NOTICES. The release archive still ships the file
  standalone (install.sh places it at
  $PREFIX/lib/antipythony/THIRD_PARTY_NOTICES); the archive file and
  the embedded copy CAN drift between releases — the binary reports
  what IT was built with (the honest provenance answer), and
  `xtask notices --check` at the release gate guards the file itself.
- Unit tests: version line carries name+version; embedded notices
  non-empty + headered.
- Usage text documents both flags.

## Gate verification (honest)

GREEN in-session (worktree, CARGO_TARGET_DIR=$PWD/target,
CARGO_BUILD_JOBS=4, single heavy lane with backoff while the T121
bisect + 04.05 agents built):
- `cargo test -p ap-cli --lib` — 4/4 (the 2 D3 CRT-lookup tests +
  2 D1 notices tests).
- `--test packaging_differential` — 7/7.
- `--test wasm_build_gate` — 3/3.
- `--test tier_mode_differential` — 3/3 (ran green earlier in-session).
- `--version` / `--notices` binary behavior verified via the same
  test-compiled surface; usage updated.

NOT re-run in-session (blocked by the permission layer mid-session;
do not retry):
- `--test ast_getattr_fence` — ALREADY VERIFIED GREEN FOR THIS EXACT
  CODE by the orchestrator earlier on this branch state:
  `AP_AST_FENCE_DEADLINE_SECS=600 cargo test -p ap-cli --test
  ast_getattr_fence` -> 2/2 passed in 211s. Not a regression risk:
  D13 touches only the CRT lookup path (unit-tested) + new flags
  (unit-tested); no ap-run/runtime code touched.
- `--test gate_watchdog`, `--test osr_gate`,
  `--test stdlib_modules_differential` — not re-run this session
  (heavy-lane contention + the fence block); none touch D13's surface.
  Last known green on the parent trees.

## GOTCHA recorded for the next agent

The worktree's `.cargo/config.toml` redirects target-dir to
`target-0304c/`, but `ast_getattr_fence.rs` and `wasm_build_gate.rs`
hardcode a LITERAL `<repo>/target/{debug,release}/` path. In a fresh
worktree run `CARGO_TARGET_DIR=$PWD/target cargo build -p ap-run
--bin ap-run-probe` + `cargo build -p ap-cli -p ap-host --bins` first,
or the fences fail with spawn-NotFound (same class as the documented
stale-test-binary gotcha in AGENTS.md).

## CI expectation for the next release.yml run

- The `fresh-machine-smoke` job's BUILD step should now PASS: the
  installed binary resolves the CRT from
  `$PREFIX/lib/antipythony/libap_aot_crt.a` (install.sh layout).
  Mark it for verification on the next release.yml run — this leaf
  does NOT edit release.yml (another agent's file, per dispatch).
- Suggested (not done, to keep the surface minimal): have release.yml
  set `AP_COMMIT_SHA=${{ github.sha }}` for the ap-cli builds so
  `--version` reports the commit.

## Files

- crates/ap-cli/src/build.rs (D3 lookup + tests)
- crates/ap-cli/src/notices.rs (NEW, D1)
- crates/ap-cli/src/lib.rs (mod notices)
- crates/ap-cli/src/main.rs (dispatch arms + usage text only)
- plans/antipythony/04-packaging-ecosystem/NEXT-D13.md (this record)
