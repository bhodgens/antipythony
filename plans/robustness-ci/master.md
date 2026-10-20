# ROBUSTNESS-CI - Master Plan

Implementation plan for every finding and recommendation in
`.hermes/audits/robustness-review-2026-10-19.md` (the "review"). Goal: turn the
repo's opt-in, single-laptop gate stack into enforced CI + tooling, without
disturbing the in-flight Plan 02 work.

This is a SIBLING tree to `plans/antipythany/`. It does not change product
semantics; it hardens gates around them. Locked decisions in
`plans/antipythany/master.md` remain binding here too (byte-exact conformance,
corpus-only-grows, file-ownership parallelism).

## Ground rules (binding for all leaves)

- Read-only audit already done; this tree implements. No finding is dropped:
  the coverage table below routes all 20 findings + the roadmap items.
- Leaves fit one 256K-context agent (repo convention).
- Every leaf gate is machine-checkable (command + exit code), run by the
  orchestrator; every leaf report carries an HONEST gap list.
- Parallel-safety: leaves touching the same file are sequenced, never
  parallelized. Ownership lines are in each brief.
- Never weaken a test or corpus case to make a gate pass (project-wide rule
  restated because CI work is exactly where people tempt themselves).
- The review's pass counts were never re-executed (static review, sibling
  mid-edit). Any leaf that needs a baseline count re-runs it first.
- GitHub Actions is the target platform. `attribution.yml` is the only
  existing workflow; keep it alive until its job lands in `ci.yml`.

## Interface contracts between children

| Contract | Value | Consumers |
| --- | --- | --- |
| CI toolchain | rustc 1.94.0 pinned via `dtolnay/rust-toolchain@master` (matches rust-toolchain.toml:6) | every workflow job |
| Reference python discovery | helper resolving `AP_CONFORM_REFERENCE_PYTHON` -> PATH `python3.14` -> version-string check (logic exists at `crates/ap-conform/src/lib.rs:90-96`); ALL Rust differential tests + pytest suites use it | 00/02, 01/*, 04/* |
| Skip policy | `AP_REQUIRE_REFERENCE=1` converts reference-missing skips into hard failures; set in every CI job that runs differential tests | 00/02, 00/03 |
| XPASS ceiling | committed integer in `tests/XPASS_CEILING`; CI fails when count exceeds it (monotone-decreasing) | 01/01 |
| Impl binaries for CI pytest | env `APX_IMPL=target/debug/ap-run-eval`, `APX_CLI="target/debug/antipythany run"`; `cargo build` before `cargo test`/`pytest` (integration tests path-math into target/debug, e.g. `crates/ap-run/tests/re_differential.rs:24-29`) | 00/03 |
| Lint policy | `[workspace.lints]` in root Cargo.toml; `clippy::unwrap_used = warn` allow-listed per crate; CI `cargo clippy -D warnings` | 02/01 |
| Golden provenance | `tests/corpus/<fam>/expected/<case>.meta` with `sys.version` + date + generator hash, written by each `gen_expectations.py`; lint fails goldens without meta | 03/01 |
| Vendored drift | `cargo run -p xtask -- stdlib-provenance --check` vs declared-deviations manifest `vendored_stdlib/DEVIATIONS.toml`; undeclared drift fails CI | 03/02 |

## Branch index and dispatch order

| Branch | Directory | Covers findings | Depends on |
| --- | --- | --- | --- |
| 00 CI core | `00-ci-core/` | #1 #2 #3 #4 #6 #19 #20 | none |
| 01 Test integrity | `01-test-integrity/` | #5 #10 #12 | 00 (needs ci.yml to host gates) |
| 02 Lint gates | `02-lint-gates/` | #7 #8 #16 | 00 (clippy job) |
| 03 Provenance | `03-provenance/` | #11 #13 #14 #15 | 00 (deny/provenance jobs) |
| 04 Fuzzing | `04-fuzz/` | #9 | 00/02 (reference discovery helper) |
| 05 CLI + docs automation | `05-cli-docs/` | #17 #18 + docs roadmap | 00 |
| 06 Module decomposition | `06-decomposition/` | size-risk item in review "larger investments" | sequenced with Plan 02 (see orchestrator) |

Dispatch waves:
- Wave A (parallel): 00/01, 00/02, 02/02, 03/03.
- Wave B (after A): 00/03, 01/01, 01/03, 01/04, 02/01, 03/01, 03/02, 05/01.
- Wave C: 01/02 (ongoing per-leaf flips), 01/05, 04/01, 04/02, 05/02, 05/03, 05/04.
- Wave D: 04/03 (nightly wiring), 05/05 (mdBook; hold until after Plan 03 docs settle).
- Wave E: 06/* only in a Plan 02 lull (see `06-decomposition/orchestrator.md`).

## Coverage table (review finding -> leaf)

| # | Sev | Finding (review) | Leaf |
| --- | --- | --- | --- |
| 1 | P1 | No CI beyond notices gate | 00/01, 00/03 |
| 2 | P1 | cargo-deny declared but never invoked | 00/01 |
| 3 | P1 | Rust differential tests skip silently w/o python3.14 | 00/02 |
| 4 | P1 | Reference interpreter resolved 3 inconsistent ways | 00/02 |
| 5 | P1 | 126 stale xfail XPASS markers (strict=False) | 01/01, 01/02 |
| 6 | P2 | Toolchain pin decorative; CI builds @stable | 00/01 |
| 7 | P2 | 149 unwrap() panic surfaces in ap-obj/ap-run | 02/01 |
| 8 | P2 | No lint policy (workspace.lints/clippy.toml/fmt) | 02/01, 00/01 |
| 9 | P2 | sre.rs 2603-loc backtracker + repr paths: zero fuzz | 04/01, 04/02, 04/03 |
| 10 | P2 | ap-gc no in-crate tests; xtask zero tests | 01/03, 01/04 |
| 11 | P2 | Goldens recorded on 3.14.7 vs contract 3.14.0, no provenance | 03/01 |
| 12 | P2 | bench_report.json churn + hardware-flaky gate | 01/05 |
| 13 | P2 | vendored_stdlib drift not machine-checked | 03/02 |
| 14 | P3 | AGENTS.md status block stale, no verified-at | 03/03 |
| 15 | P3 | global core.hooksPath kills repo-local hooks | 03/03 |
| 16 | P3 | Importer path joins lack segment guard | 02/02 |
| 17 | P3 | Hand-rolled testability bins; ap-parse-check missing | 05/01, 05/02 |
| 18 | P3 | No cargo-doc gate, broken links pass | 05/03 |
| 19 | P3 | corpus_traceability lints CI-invisible (cheapest gate) | 00/01 |
| 20 | P3 | T-15:53 xfails "CI runner config existence" | 00/01, 01/02 (first strict flip) |
| R | — | Docs roadmap: clap-generated CLI docs, mdBook | 05/04, 05/05 |
| R | — | Mutation testing | deferred by review; NOT a leaf (tracked below) |
| R | — | interp.rs/builtins_modules.rs decomposition | 06/01, 06/02 |

Deliberate non-items: branch-protection enablement is a GitHub UI action (not
a code leaf); 00/04 writes the recommended settings doc for the human.

## Deferred / queue

- Mutation testing (cargo-mutants): review says revisit when 02.05 adds JIT
  paths goldens under-exercise. No leaf. Entry lives here.
- mdBook deploy target: GitHub Pages; hold until docs settle post-Plan 03.
- RESOLVED (00.01 step 0, 2026-10-19): setup-python@v5 resolves "3.14" GA
  natively (actions/python-versions manifest ships 3.14.6-3.14.8 linux).
  fast-lints pins "3.14.7" for the 03.02 upstream-sha manifest (a2da3a4).

## Completion tracking

Updated by the orchestrator after each leaf review. All leaves NOT STARTED.

| Leaf | Status |
| --- | --- |
| 00.01 ci.yml core jobs | COMMITTED (717bc9c) |
| 00.02 reference discovery + AP_REQUIRE_REFERENCE | COMMITTED (7c8358d) |
| 00.03 conformance matrix jobs | COMMITTED (a881906) - remote green pending push |
| 00.04 branch-protection doc | NOT STARTED |
| 01.01 XPASS ceiling gate | COMMITTED (0137c64) - ceiling 74 |
| 01.02 ledger burn-down (ongoing) | IN PROGRESS - T-01 seam flip 22e73e8 |
| 01.03 ap-gc unit tests | COMMITTED (3635a6f) - 23 tests |
| 01.04 xtask notices tests | COMMITTED (1c16009) - 8 tests |
| 01.05 bench trend-only | NOT STARTED |
| 02.01 workspace.lints + clippy gate | COMMITTED (4c8d08b) - CI -D warnings flip pending |
| 02.02 importer segment guard | COMMITTED (6dae48c) |
| 03.01 golden .meta provenance | COMMITTED (8db313c) - 232 metas |
| 03.02 stdlib-provenance xtask | COMMITTED (5b39f29) + CI-wired a2da3a4 |
| 03.03 AGENTS.md hygiene + hooks note | COMMITTED (6b67db0) |
| 04.01 proptest re differential | COMMITTED (5de2caa) - RF-1..7 routed to Plan 02 |
| 04.02 proptest repr differential | COMMITTED (5de2caa) - F0402-1/2 routed |
| 04.03 nightly fuzz workflow | COMMITTED (e899c9e) |
| 05.01 clap migration (4 bins) | COMMITTED (1bd757d) |
| 05.02 ap-parse-check seam bin | COMMITTED (846f377 bin + 22e73e8 T-01 flip) |
| 05.03 rustdoc gate + missing_docs surfaces | COMMITTED (6c899b1) - ap-run via 05.03b |
| 05.04 generated CLI reference | COMMITTED (5b39f29) + CI-wired a2da3a4 |
| 05.05 mdBook site | NOT STARTED |
| 06.01 builtins_modules.rs decomposition | NOT STARTED |
| 06.02 interp.rs decomposition | NOT STARTED |

## Integration review plan

After each wave: the orchestrator (main model) checks:
- ci.yml jobs all green on a push; no job silently skips (grep for
  `continue-on-error` — none permitted).
- `AP_REQUIRE_REFERENCE=1` actually flips a missing-reference run from skip
  to red (run one locally with PATH scrubbed as a fresh probe).
- XPASS + corpus pass counts moved in the allowed direction only.
- THIRD_PARTY_NOTICES still drift-clean (`cargo run -p xtask -- notices
  --check`).
- This tree adds no product-semantics changes: if a leaf needs one, it stops
  and files it against the antipythany plan tree instead.
