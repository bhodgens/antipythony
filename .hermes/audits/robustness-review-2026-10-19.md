# antipythany — Robustness & Maintainability Review (2026-10-19)

Read-only engineering review of the Rust monorepo at `/Users/caimlas/git/antipythany`.
Scope: CI/CD, test strategy, documentation automation, code-quality gates,
reproducibility/maintainability, security, onboarding. Evidence is cited as
`path:line`. No repo files were modified; findings were gathered by static
inspection only (`git log/status`, `ls`, `grep`, `find`, file reads) — no
`cargo test`/`pytest` was executed because a sibling agent is mid-edit in the
working tree (`git status`: modified `crates/ap-gc/src/gcmod.rs`,
`tests/t02_ir_shape/test_t02_ir_shape.py`; commit `7a144b9 "re engine complete"`
landed while the review ran, so counts below are as of HEAD `b8eb72b`).

---

## Executive summary

antipythany has an unusually strong *testing contract* — byte-exact differential
goldens, a spec-citation traceability lint, pinned-env runner discipline, and a
regeneration-never-hand-edit rule (tests/README.md:1–55). What it does **not**
have is enforcement: the entire gate stack is opt-in on one laptop. `.github/`
contains exactly one workflow (`attribution.yml`, the notices drift gate); no CI
runs `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt --check`, pytest,
cargo-deny, or the corpus. master.md:45 promises a cargo-deny + cargo-about CI
gate; deny.toml exists and is well-argued but nothing invokes it — the same
"declared but never wired" class of bug the 2026-10-17 bughunt found for the
notices gate (attribution.yml header comment).

Secondary risks, in order: (1) the toolchain pin (`rust-toolchain.toml` → 1.94.0)
is documentation-only and the one existing workflow builds with `@stable`, so CI
and dev machines can silently diverge on a project whose exit gate is byte-exact
output; (2) the reference interpreter is resolved three different ways
(env-var-with-homebrew-fallback in pytest suites, hardcoded `const REF`
strings in two Rust differential tests, a `python3` fallback in others that
violates tests/README rule 5), and Rust differential tests *skip silently* when
python3.14 is absent — a CI runner without it would report green; (3) 126 stale
`xfail(strict=False)` markers currently XPASS on every run (commit 09c6887,
`plans/antipythany/02-native-backend/STALE-XFAIL-LEDGER.md`) — a regression of
those behaviors goes green instead of red; (4) 149 `.unwrap()` sites in the two
hot crates (ap-obj: 88, ap-run: 61) are panic surfaces in a runtime whose spec
says stderr bytes must match CPython — a panic prints a Rust backtrace, i.e. a
guaranteed conformance failure, not a controlled `SystemError`; (5) three
crates of 48.7k src lines (ap-gc, xtask, and both testability-bin crates'
hand-rolled arg parsing) plus the brand-new hand-written regex backtracker
(`crates/ap-run/src/sre.rs`, 2,603 lines) have no property/fuzz coverage at all,
even though fuzzing `re` and the parser is the highest-value differential
hunting ground this project has (any input where CPython and ap-run disagree is
a shipped bug).

The fix is sequenced and cheap: one CI file that mirrors the local gate stack
(~half a day), unify reference discovery (~1–2 h), then flip the xfail ledger to
strict per-leaf, add `workspace.lints` + clippy gates, and a provenance script
for vendored_stdlib. Larger investments (proptest corpus, module decomposition of
builtins_modules.rs/interp.rs, mdBook docs site) follow.

---

## Findings table

| # | Sev | Finding | Evidence | Recommendation |
|---|-----|---------|----------|----------------|
| 1 | P1 | No CI besides the notices drift gate. Nothing builds/tests/lints Rust or runs pytest/corpus/traceability in CI; 91 commits since 2026-10-15 landed unverified by any remote gate. | `ls .github/workflows/` → `attribution.yml` only | Ship the 5-job pipeline sketched below (`ci.yml`), mirroring tests/README.md:64–70 "Running" section. |
| 2 | P1 | master.md contract promises `cargo-deny` in CI; deny.toml is configured but never invoked. Same declared-but-unwired class the bughunt caught for notices (attribution.yml:3–5 admits it). | `plans/antipythany/master.md:45`; `deny.toml`; no workflow references it | Add `cargo deny check licenses bans sources` job (dtolnay or taiki-e install-action). |
| 3 | P1 | Rust differential tests **skip silently** when python3.14 is missing → false-green in any env lacking it. | `crates/ap-run/tests/import_differential.rs:20` (candidate loop then expect), `fix_pass_differential.rs:20`; `crates/ap-parse/tests/corpus.rs` "Skips when python3.14 is unavailable" | Keep skip-on-absent for local convenience, but gate it: honor `AP_REQUIRE_REFERENCE=1` (set in CI) to hard-fail instead of skip. |
| 4 | P1 | Reference interpreter resolved 3 inconsistent ways; two suites hardcode a Homebrew path; violates the suites' own rule 5 ("never a bare `python3` fallback") in Rust tests. | `tests/t08_tier1/test_t08_tier1.py:81` (env + `/opt/homebrew/bin/python3.14` default); `crates/ap-run/tests/method_surface_differential.rs:13` + `re_differential.rs:12` (`const REF: &str = "/opt/homebrew/bin/python3.14"`); `classes_differential.rs:15`, `metaclass_enum_differential.rs:17` fall back to `python3` | Centralize: a `reference_python()` helper reading `AP_CONFORM_REFERENCE_PYTHON`, then PATH lookup with the existing `--version`-string 3.14 check (already correctly implemented at `crates/ap-conform/src/lib.rs:90–96`); reuse it from every Rust differential test. CI sets the env var. |
| 5 | P1 | 126 stale-xfail XPASS markers with `strict=False`: covered behaviors that regress would report "expected failure", i.e. green. | commit `09c6887` + `plans/antipythany/02-native-backend/STALE-XFAIL-LEDGER.md`; `tests/t08_tier1/test_t08_tier1.py:106`; TRACKING.md:7 already reports 25→126 xpass growth | Flip per-test to strict when each owning leaf (01.02/01.03/01.04/02.05) lands, as the ledger says; add CI job that runs `pytest -q --xfail-strict-report` and fails when XPASS count grows past a committed ceiling. |
| 6 | P2 | Toolchain pin decorative + CI mismatch: rust-toolchain.toml self-documents "Homebrew cargo ignores it"; the one workflow uses `@stable`. Byte-exact projects are sensitive to codegen/formatting drift between rustc versions. | `rust-toolchain.toml:1–7`; `attribution.yml:15` | CI pins `dtolnay/rust-toolchain@master toolchain: 1.94.0`; publish the exact version check as a CI step (`rustc -vV` assertion); note (not fix here) that local dev is Homebrew rustc 1.94.0. |
| 7 | P2 | Panic-surface audit: 149 `unwrap()`s + 3 `panic!`s in src/ concentrated in the crates closest to the byte-exact gate. | grep counts: ap-obj/src 88, ap-run/src 61, ap-parse/src 3; e.g. `ap-run/src/interp.rs:1175,3465` | Don't mass-rewrite. Add `[workspace.lints]` with `clippy::unwrap_used = "warn"` allow-listed per crate, so *new* unwraps in ap-run/ap-obj need justification; run a clippy job in CI. |
| 8 | P2 | No lint policy at all: no `[workspace.lints]` in root Cargo.toml, no clippy.toml, no `missing_docs` anywhere, no rustdoc gate; `cargo fmt`/`clippy` never enforced. | `Cargo.toml` (no lints key); `ls clippy.toml` → absent | Add workspace lints table + a CI `clippy -- -D warnings` job with a scoped allow-list; enforce `cargo fmt --check`. |
| 9 | P2 | New hand-written regex backtracker (`sre.rs`, 2,603 loc + `sre_tables.rs` 898 loc) has only hand-written probes; zero property/fuzz coverage. Same for parser→IR lowering and float/int repr (`ap-obj/src/float_repr.rs`). | `crates/ap-run/src/sre.rs`; `crates/ap-run/tests/re_differential.rs` (fixed probe list) | Add `proptest`-based differential fuzzing as dev-dependency: generate random patterns/inputs, assert byte-exact stdout vs reference python3.14 (skip under `cargo test` CI fast lane, run nightly). Highest bug-yield-per-effort of anything in this report. |
| 10 | P2 | ap-gc (855-loc tracing GC, `gcmod.rs`) has **no test dir**; its differential coverage lives in ap-run (`gc_differential.rs`). Unit-level GC invariants (cycle collection, weakref bridge, allocation accounting) untested in-crate. | `crates/ap-gc/` — no `tests/`; `grep cfg(test)` finds no gc files | Add `crates/ap-gc/tests/gc_invariants.rs` + intra-crate unit tests; also xtask has zero tests despite owning the only live CI gate (`crates/xtask/src/main.rs`). |
| 11 | P2 | Reference-version drift unresolved: contract pins CPython 3.14.0 (C-01) but every golden was recorded on 3.14.7; TRACKING open-decision #4 acknowledges the debt. No machine record of which interpreter produced a pin. | `docs/compat-contract.md:29–32`; `plans/antipythany/tests/TRACKING.md` item 4 | Have each `gen_expectations.py` write a `expected/<case>.meta` with `sys.version` + date; add a lint that goldens without meta fail. Decides 3.14.0-vs-3.14.7 re-verification mechanically later. |
| 12 | P2 | Benchmark report committed with wall-clock times and churned (7 commits touch it); a 0.94 geomean against a 1.5 gate will be flaky across runner hardware, and the numbers already fail the gate pre-02.05 (by design — adaptive==tier0 today). | `tests/t08_tier1/bench_report.json` (gate line, `geomean_speedup_conservative: 0.9418`) | Keep it as a trend artifact, never a CI pass/fail; if 02.05 gates on ≥1.5×, run the bench only on a self-hosted/ci-tagged fixed hardware job, ratio-only comparison. |
| 13 | P2 | vendored_stdlib provenance is manual: 71 tracked files copied from CPython 3.14.7 with *documented deliberate deviations* (commit `1cb6250`), but no tool to distinguish deviation-from-upstream vs accidental drift. | `git ls-files vendored_stdlib \| wc -l` = 71; commit `1cb6250 docs(license): document vendored_stdlib deviations` | `xtask stdlib-provenance --check`: fetch (or local) CPython 3.14.7 `Lib/` files, sha256-diff, report only files not in a declared-deviations manifest. Fail CI if an undeclared file changes. |
| 14 | P3 | AGENTS.md status block is already stale: "80 commits / rust 247 / pytest 1272 / corpus 150/197" vs HEAD with 91+ commits since Oct 15 and the re engine just landed. Standing convention says update this block; it will rot between dispatch waves. | `AGENTS.md:42–46` vs `git log --oneline --since=2026-10-15 \| wc -l` = 91 | Point the block at `plans/**/PROGRESS.md` + a `xtask status` one-liner (counts via `cargo test --no-run`? no — keep prose, add last-verified date + commit sha so staleness is visible at a glance). |
| 15 | P3 | Global `core.hooksPath = ~/.githooks` (LFS hooks only) means repo-local pre-commit hooks silently never fire; any future repo hook (fmt, cites-header check) is a no-op on this machine. | `git config core.hooksPath`; `ls ~/.githooks` → no repo gates | Prefer CI gates over git hooks here; if hooks wanted, document `git config core.hooksPath .githooks` in onboarding. |
| 16 | P3 | Importer path hygiene: `find_submodule`/`find_on_path` join module name segments onto dir paths without rejecting `..`, `/`, or absolute-ish segments; reachable only if a dotted-name path can carry them (parser-dotted names can't, but `__import__`-with-computed-string could, once builtins accept arbitrary str names). | `crates/ap-run/src/importer.rs:185–206` (`bp.join(name)`, no guard); name flows from module-exec dotted walk `:413–470` | Cheap invariant: in `find_on_path`/`find_submodule`, reject any segment equal to `""`, `"."`, `".."` or containing `\\`/`/` (CPython raises `ImportError: invalid module name` — match the bytes). Add a corpus case. |
| 17 | P3 | Testability CLIs are hand-rolled arg loops with no `--help`; only ap-conform uses clap. T-01's gate seam `ap-parse-check` **does not exist** yet (TRACKING decision #3) while suites read `AP_PARSE_CHECK`. | `crates/ap-cli/src/bin/ap_run_eval.rs:15` manual loop; `crates/ap-parse/src/bin/ap_verify_dump.rs:27` (positional only); `find crates -name '*parse_check*'` → none; `tests/t01_parse_diagnostics` reads `AP_PARSE_CHECK` | Migrate the four eval/dump bins to clap (uniform `--help`, version, errors) — small and self-contained; then add `ap-parse-check` as T-01's declared seam or amend the suite. |
| 18 | P3 | No cargo-doc policy or docs site: `docs/` is hand-maintained markdown (spec pack + design notes) with good content but no index/build; rustdoc has no lint gates (broken intra-doc links pass). | `ls docs/` (no mkdocs/book.toml); no `[lints.rust]` broken_intra_doc_links | Cheap: CI `cargo doc --no-deps -p ap-obj -p ap-ir -p ap-parse -p ap-run` with `RUSTDOCFLAGS=-D warnings`. Later: mdBook over docs/spec+compat-contract as the public site. |
| 19 | P3 | Corpus/expected inventory is healthy but CI-invisible: 270 corpus .py + 735 pinned files across families; nothing runs `tests/corpus_traceability/test_corpus_traceability.py` remotely despite it being stdlib-only and sub-second — the cheapest true gate in the repo. | `find tests/corpus -name '*.py' -not -path '*expected*' \| wc -l` = 270; expected files = 735; `tests/corpus_traceability/test_corpus_traceability.py:1–10` | Put traceability lint + corpus-rule lints (Cites header, salted-hash) in the fast pytest job that needs no built binary. |
| 20 | P3 | No release/versioning story yet: ap-cli is a stub with `run`-mode bins; T-15's release gates (checksums, install script, `--version --notices` embedding, CI runner config existence!) are all xfail-by-design. Fine until Plan 04, but T-15:53 literally xfails "CI runner config existence" — CI today is its own precondition. | `tests/t15_release/test_t15_release.py:44–58` | Land ci.yml now (this report), then un-xfail T-15's CI-config test as the first strict flip. |

---

## Proposed CI pipeline

Design constraints found in-repo:
- CI must build with **rustc 1.94.0** (rust-toolchain.toml:6) — use
  `dtolnay/rust-toolchain@master` with `toolchain: 1.94.0`, not `@stable`.
- Differential suites need **CPython 3.14** on PATH (`crates/ap-conform/src/lib.rs:91`
  looks up `python3.14` then `python3` but only accepts a `--version` reporting
  3.14.x — so `actions/setup-python@v5` with `python-version: "3.14"` satisfies
  the contract *and* the version check; also export `AP_CONFORM_REFERENCE_PYTHON`
  and `APX_IMPL`/`APX_CLI` (read at `tests/t05_values/test_t05_values.py:77`,
  `tests/t11_stdlib/test_t11_stdlib.py:109`) pointing at the built binaries.
- Rust integration tests reference `target/debug/ap-run-eval` via workspace-root
  path math (`crates/ap-run/tests/re_differential.rs:24–29`), so a workspace
  `cargo build` must precede `cargo test` (or run in one job sequentially).
- Env flag `AP_REQUIRE_REFERENCE=1` makes finding #3's skips into failures in CI.

```yaml
# .github/workflows/ci.yml  (sketch)
name: ci
on:
  push: { branches: [main] }
  pull_request:

env:
  AP_REQUIRE_REFERENCE: "1"
  CARGO_TERM_COLOR: always

jobs:
  fast-lints:                      # no compile needed; seconds
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "3.14" }
      - run: pip install pytest
      - name: spec-citation + corpus-rule lints (stdlib only)
        run: pytest tests/corpus_traceability tests/t01_parse_diagnostics -q -m "not needs_bin"
      - name: notices drift gate (moved from attribution.yml)
        run: cargo run -p xtask -- --version >/dev/null 2>&1 || true  # see build-and-test; keep job small

  rust:                            # fmt / clippy / build / test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: "1.94.0"
          components: rustfmt, clippy
      - uses: actions/setup-python@v5
        with: { python-version: "3.14" }   # differential Rust tests need it
      - uses: Swatinem/rust-cache@v2       # caches target/ keyed on Cargo.lock
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo build --workspace       # bins consumed by integration tests
      - run: cargo test --workspace
      - run: cargo doc --no-deps -p ap-obj -p ap-ir -p ap-parse -p ap-run
        env: { RUSTDOCFLAGS: "-D warnings" }

  deny:                            # master.md:45 promise
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: taiki-e/install-action@v2
        with: { tool: cargo-deny }
      - run: cargo deny check

  conformance:                     # byte-exact corpus gate
    needs: rust
    strategy:
      matrix: { os: [ubuntu-latest, macos-14] }   # macOS mirrors dev env
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with: { toolchain: "1.94.0" }
      - uses: actions/setup-python@v5
        with: { python-version: "3.14" }
      - uses: Swatinem/rust-cache@v2
      - run: cargo build --workspace
      - run: pip install pytest
      - name: pytest differential track
        env:
          APX_IMPL: "target/debug/ap-run-eval"
          APX_CLI:  "target/debug/antipythany run"
        run: pytest tests/ -q --no-header

  nightly-fuzz:                    # proptest differential (finding #9), nightly cron only
    if: github.event_name == 'schedule'
    runs-on: ubuntu-latest
    steps: [ ... cargo test -p ap-run --release --features proptest differential_fuzz ... ]
```

Notes: `macos-14` runners cost ~10× — run the conformance matrix there only on
main, PRs on ubuntu. Drop `attribution.yml` once the deny+notices jobs land
(T-15:53 xfails until a CI config exists — this satisfies it).

---

## Test strategy roadmap

1. **Make skips loud (week 1).** `AP_REQUIRE_REFERENCE=1` → reference-missing is
   a failure, not a skip (finding #3), and one shared discovery helper (finding
   #4). Without this, every CI green below is untrustworthy.
2. **XPASS ledger burn-down (per-leaf).** The ledger already names owners; each
   leaf landing flips its tests to `strict=True`. Add a committed ceiling number
   in CI so the XPASS count can only shrink. 126 stale markers are ~13% of the
   suite watching nothing.
3. **Property/differential fuzzing (highest yield).** New regex engine
   (`sre.rs` 2,603 loc), `float_repr.rs`/`int.rs` repr paths, and parser
   diagnostics are exactly the domains where random-input byte-diff finds bugs
   no hand-probe list covers. proptest generating (pattern, subject) pairs, run
   both sides via subprocess, compare stdout bytes; same shape for
   `repr(float)` over bit-random doubles. Nightly job, not PR-blocking.
4. **Crate-level gaps.** ap-gc gets in-crate unit tests (cycle break, weakref
   bridge ordering — `crates/ap-gc/src/weakbridge.rs` is a global-state design
   that deserves its own tests); xtask gets a notices-determinism test (the
   header at `crates/xtask/src/main.rs:26` *claims* sorted determinism — assert
   it).
5. **Mutation testing: defer.** cargo-mutants on ap-obj/ap-run is feasible in
   principle but the byte-exact corpus already does semantic oracle work
   mutation testing is meant to approximate; ROI is low vs. fuzzing. Revisit
   when tier-1 (02.05) adds JIT paths that goldens under-exercise.
6. **Benchmarks:** keep `bench_report.json` trend-only (finding #12); when
   02.05 lands, gate the ≥1.5× ratio on fixed hardware only.

## Documentation automation plan

1. **rustdoc in CI first** (`cargo doc -D warnings`): free intra-doc-link and
   code-block verification over the 8 real crates; the crates are heavily
   module-commented already (e.g. `crates/ap-run/src/lib.rs` head notes), so the
   marginal cost is near zero.
2. **`missing_docs`: keep off for internals** — correct call for a compiler
   codebase of 48.7k loc; enforce only on the 3 public-facing API surfaces
   (ap-obj value model, ap-ir, ap-conform lib) via crate-level
   `#![warn(missing_docs)]`, not workspace-wide.
3. **CLI reference from the code**: migrate the 5 testability bins to clap
   (finding #17) — `--help` output then *is* the documentation; generate
   `docs/testability-clis.md` in xtask from `Command::render_help`.
4. **mdBook over docs/** last: `src/SUM.md` → spec pack 01–06 +
   compat-contract + attribution + path-to-targets; deploy via GitHub Pages
   from the fast-lints job. Do not migrate design-notes (pon-study, gc-mmtk)
   into the book body — appendix or leave in-tree.
5. **AGENTS.md staleness (finding #14):** add "verified at commit <sha> on
   <date>" line; counts belong in PROGRESS.md which each wave already updates.

## Quick wins (< 1 h each) vs larger investments

**Quick wins**
- Add `AP_REQUIRE_REFERENCE` hard-fail flag to the ~6 Rust test files + conftest-style helper. (fixes #3)
- Unify `REF` consts → env-var lookup with version check (`crates/ap-conform/src/lib.rs:90` logic is already written — reuse it). (#4)
- `cargo fmt --all --check` + `cargo clippy -D warnings` as one CI job, pinned 1.94.0. (#1,#8)
- `cargo deny check` job. (#2)
- pytest job for `tests/corpus_traceability` (no build needed). (#19)
- CI `cargo doc -D warnings`. (#18 first half)
- Segment-guard in `find_submodule`/`find_on_path` + one corpus case. (#16)
- AGENTS.md verified-at line. (#14)

**Larger investments (sequenced)**
- Full `ci.yml` above with caching + macOS matrix (~0.5 day). (#1)
- clap migration of the 4 eval bins + `ap-parse-check` seam bin (~0.5 day). (#17)
- XPASS strict flip ledger burn-down (ongoing, per-leaf). (#5)
- proptest differential fuzz harness for re + repr (~1–2 days). (#9)
- `xtask stdlib-provenance` + `expected/*.meta` pin-provenance (~1 day). (#11,#13)
- `workspace.lints` rollout with `unwrap_used` warn + per-crate allowlists (~1 day incl. triage). (#7)
- Decompose `builtins_modules.rs` (11,629 loc) / `interp.rs` (6,590 loc) by module family — schedule around 02.08-full p2, not before. (size risk)
- mdBook site (~1 day, after docs settle post-Plan-03).

---

## What could NOT be verified (read-only constraint)

- Actual pass counts (`rust 247/0`, `pytest 1272/0`, corpus 150/197) — not re-run; sibling agent mid-edit; counts quoted from AGENTS.md/commit messages.
- Whether `cargo build` succeeds at HEAD including the uncommitted `sre.rs` state.
- GitHub-side behavior (branch protection, runner availability of setup-python 3.14 on the account's tier).
- `xtask notices --check` currently passing (T-15 authoring observed drift; not re-executed).
- Exact semantics of the ~61 ap-run / ~88 ap-obj unwraps (spot-checked only; no full classification of test-only vs reachable paths).
