# Branch 00 PROGRESS

## 00.01 IMPLEMENTED NOT COMMITTED (2026-10-19, leaf agent)

### What landed
- **`.github/workflows/ci.yml` (new)** — core jobs per master.md contract:
  - `fast-lints`: checkout → setup-python@v5 "3.14" → `pytest
    tests/corpus_traceability -q` (finding #19, C-02 traceability lint) →
    pinned rustc → `cargo run -p xtask -- notices --check` (the
    attribution.yml gate, duplicated here so orchestrator can retire that
    file with a pure delete).
  - `rust`: `dtolnay/rust-toolchain@master` toolchain "1.94.0" + components
    rustfmt,clippy → explicit `rustc -vV | grep "rustc 1\.94\.0"` assertion
    step (finding #6 "pin is decorative") → setup-python "3.14" +
    `AP_CONFORM_REFERENCE_PYTHON` export for 00.02's helper → Swatinem/
    rust-cache@v2 → `cargo fmt --all --check` (blocking) → clippy
    **informational, exit-0 wrapper** emitting a `::warning::` with the
    count (strict `-D warnings` is leaf 02.01's job — comment in file;
    tree has ~456 clippy warnings today per finding #7, a hard gate would
    be red-on-merge) → `cargo build --workspace` → `cargo test
    --workspace` in one job (integration tests path-math into
    target/debug).
  - `deny`: `taiki-e/install-action@v2` tool cargo-deny → `cargo deny
    check` (finding #2, master.md:45 promise).
  - `conformance`: **stubbed as a comment block only**, TODO references
    leaf 00.03 — not half-wired, because without 00.02's
    AP_REQUIRE_REFERENCE helper a runner lacking python3.14 reports green
    (finding #3). Workflow-level `AP_REQUIRE_REFERENCE: "1"` set now
    (comment notes it activates with 00.02; harmless today).
- Triggers: `push: branches [main]` + `pull_request` (orchestrator.md
  design; the leaf-00.01 "Gate" line wants main+PR).
- attribution.yml left untouched per brief (orchestrator retires it).

### Step-0 answer (setup-python@v5 + python "3.14")
VERIFIED YES. Evidence (2026-10-19): actions/python-versions
`versions-manifest.json` (fetched via `gh api`) contains 3.14 GA builds
through **3.14.8** for linux platform_version 22.04/24.04/26.04, x64 and
arm64; actions/setup-python's own `test-python.yml` CI matrix runs
`3.14.6` and bare `3.14` on ubuntu-latest. Latest v5 tag is v5.5.0
(`gh api repos/actions/setup-python/releases/latest` shows the line is now
at v7, but the master.md contract names @v5 and v5 fully supports "3.14" —
no prerelease flag, no setup-uv fallback needed).

### Gates run locally (what could be run)
| Gate | Command | Result |
| --- | --- | --- |
| YAML syntax | `python3 -c "import yaml; yaml.safe_load(...)"` (system python3, pyyaml present) | OK — parses; jobs = fast-lints, rust, deny; `on` = push[main] + pull_request |
| fmt gate | `cargo fmt --all --check` (working tree) | **NON-ZERO — but every diff is in crates/ap-run/src/{builtins,builtins_modules,gcsupport,interp,methods}.rs + threading.rs/threading_call.rs: the sibling Plan-02 threading leaf's mid-edit untracked/dirty files. Zero fmt diffs in any other file. Not mine to fix (ownership rule).** CI will run fmt on the committed tree; if the orchestrator commits the sibling's unformatted WIP alongside, the rust job goes red on fmt — flag to orchestrator. |
| notices drift | `cargo run -p xtask -- notices --check` | OK (151 deps, up to date) |
| corpus lint | `/opt/homebrew/bin/python3.14 -m pytest tests/corpus_traceability -q` | 1 passed |
| clippy scale | `cargo clippy --workspace --all-targets \| grep -c '^warning:'` | 456 warning lines (informational job design justified) |
| cargo deny | **NOT RUN** — cargo-deny binary absent locally; brief forbids installing. | deferred to first CI run |
| actionlint | not available locally; not installed | YAML hand-validated + structure check only |

### HONEST gaps
1. **No remote green run yet** — brief says do not push; the
   orchestrator.md "Gate" (branch push + `gh run watch`, one deliberate
   red per job) is an orchestrator-committed step, not done by this leaf.
2. cargo-deny never executed against this repo — `deny.toml` validity
   unproven until the deny job first runs remotely.
3. clippy is informational-only by design (02.01 flips it); the
   exit-0 wrapper is the one permitted soft spot and is commented.
4. `cargo fmt` evidence is from a dirty working tree (sibling edits);
   committed-tree fmt was not isolated because extracting HEAD would have
   meant staging/`git archive` probes that were denied/blocked this
   session.
5. rust-cache action version, taiki-e/install-action major, and runner
   availability of rustc 1.94.0 in dtolnay's dist are standard-but-unrun;
   first CI green is the real proof.
6. `AP_CONFORM_REFERENCE_PYTHON` export falls back to `command -v python`
   (setup-python's default alias) — 00.02's helper must validate the
   `--version` string, which the master contract says it does.
7. macos conformance + nightly fuzz jobs absent by design (00.03 / 04.03).

## 00.02 IMPLEMENTED NOT COMMITTED (2026-10-19, leaf agent)

### Helper design chosen
Per-crate `tests/common/mod.rs` duplication (NO new path-dep crate, per brief
preference). Created `crates/ap-run/tests/common/mod.rs` exposing
`reference_or_skip() -> Option<&'static str>` + internal `is_python_314()`:
1. `AP_CONFORM_REFERENCE_PYTHON` accepted ONLY if `--version` reports Python
   3.14 (bad value -> stderr warning + fall through);
2. PATH `python3.14` with the same version-STRING check (stdout+stderr
   concatenated, mirroring ap-conform/src/lib.rs:101-117);
3. nothing found + `AP_REQUIRE_REFERENCE=1` -> `panic!("reference interpreter
   not found (AP_REQUIRE_REFERENCE=1)")` (skip-to-fail, finding #3);
4. nothing found + env unset -> prints `SKIP: python3.14 not found` to stderr,
   returns None; each suite's runner fn returns early (skip, green, local
   convenience preserved). Resolution cached in a `OnceLock`. NO bare
   `python3` fallback anywhere (tests/README rule 5), NO hardcoded
   /opt/homebrew paths in code.

### Files converted (all crates/ap-run/tests/)
- `common/mod.rs` (NEW helper)
- `re_differential.rs` — was `const REF` hardcode
- `method_surface_differential.rs` — was `const REF` hardcode
- `classes_differential.rs` — was local `ref_python()` w/ `python3` fallback
- `metaclass_enum_differential.rs` — same fallback chain (2 sites; also fixed
  a pre-existing duplicated run_reference block in warnings_warn_runs by
  threading py through both)
- `metaclass_arity_differential.rs` — same fallback chain
- `gc_differential.rs` — same fallback chain
- `name_resolution_differential.rs` — same fallback chain
- `import_differential.rs` — was spawn-only probe (`-c pass`, NO version
  check) on [homebrew, python3.14]; 3 test-level + 1 helper-level call sites
- `fix_pass_differential.rs` — same spawn-only probe; helper-level call site
crates/ap-parse/tests/*: grepped — ZERO python references there (the leaf
brief's citation of corpus.rs "Skips when python3.14 unavailable" is stale;
corpus.rs is pure-parse). Nothing to convert; no helper copy needed.

### Gates executed (all against target/debug binaries, cargo/rustc symlinked
into a scratch PATH dir; PATH=scratch:/usr/bin:/bin so /opt/homebrew is gone)
1. `cargo test -p ap-run` (full, normal env): ALL green — 42+6+29+2+1+1+6+34
   +24+42 passed, 0 failed (1 ignored = pre-existing BUG-C #[ignore]).
2. require mode: `PATH-scrubbed AP_REQUIRE_REFERENCE=1 cargo test -p ap-run
   --test {re,import,classes,fix_pass}_differential` -> exit 101 each,
   canonical message "reference interpreter not found
   (AP_REQUIRE_REFERENCE=1)" on every test (42/6/29/2 panic lines); also
   verified metaclass_enum/arity, gc, name_resolution, method_surface all
   exit 101 with the message.
3. skip mode: `PATH-scrubbed` (no env var) -> exit 0, "ok" results;
   `-- --nocapture` shows 42x "SKIP: python3.14 not found" in re_differential.
4. ladder step 1: `AP_CONFORM_REFERENCE_PYTHON=/opt/homebrew/bin/python3.14`
   with PATH scrubbed + REQUIRE=1 -> 42 passed (env var honored).
5. version rejection: env var pointing at /usr/bin/python3 (3.9.6), PATH
   scrubbed, REQUIRE=1 -> warning + hard fail (never diffs vs wrong version).
6. `rustfmt --check` on every touched file: clean. `cargo check -p ap-run
   --tests`: no errors.

### HONEST gaps
1. pytest suites NOT touched (ownership rule "Do NOT touch tests/*.py";
   adding AP_REQUIRE_REFERENCE honoring there is not trivial — they have no
   skip-on-missing-reference path at all, they'd FileNotFoundError, so a
   guard must be designed per-suite; already honor
   AP_CONFORM_REFERENCE_PYTHON with homebrew default, which matches the
   master contract's "default fallback keeps the Homebrew path only when no
   env is set"... the default homebrew path itself remains for leaf 00.03 /
   01 branches to revisit).
2. `crates/ap-obj/tests/cpython_differential.rs` (spawn-only probe, no
   version check, no homebrew/PATH ladder) and `crates/ap-cli/tests/
   stdlib_modules_differential.rs` (homebrew+PATH ladder with version check,
   no env-var/REQUIRE support) are inconsistent too but OUTSIDE my ownership
   (ap-run/ap-parse tests only). They need the same helper copy — file for
   the orchestrator (small mechanical follow-up; helper doc comment already
   tells converters to copy common/mod.rs).
3. `crates/ap-conform/src/lib.rs` find_reference_interpreter still carries
   its own `python3` fallback candidate (read-only for this leaf; acceptable
   because its version check rejects non-3.14, but a bare-python3 that
   reports "Python 3.14" from a weird alias would pass — benign).
4. The skip-mode "green" is honest-but-empty by design (finding #3 accepted
   trade for local convenience); only AP_REQUIRE_REFERENCE=1 (CI env in
   00.01's ci.yml) turns it into the enforcing gate.
5. Did not commit or git add anything (per brief). Working tree also contains
   sibling Plan-02 src/ edits; my gates ran against the current tree's
   target/debug binaries which include those edits — ap-run lib tests passed
   regardless, so no interference observed.

---

## Leaf 00.03 - conformance matrix job (2026-10-20)

Implemented `.github/workflows/ci.yml` conformance block (replaced the 00.01
stub comment): `conformance-matrix` axis job + `conformance` matrix job.

### Design decisions
- **macOS gating:** the brief's `if: github.event_name == 'push' && ...` at
  job level is NOT expressible — job-level `if` is evaluated before the
  matrix context exists (actions/runner issue #1985; GitHub docs confirm
  "if operates before matrix"). A job-level if can only skip ALL OSes or
  none. Solution: tiny `conformance-matrix` ubuntu job emits the OS axis as
  JSON (`["ubuntu-latest"]` for PRs/branch pushes,
  `["ubuntu-latest","macos-14"]` for main pushes) consumed via
  `matrix.os: ${{ fromJSON(...) }}`. Cost note (~10x macOS runner, audit
  line "macos-14 runners cost ~10x") is a comment in the pick step.
- **Env names verified against suites (grep evidence, not brief guesses):**
  - `APX_IMPL="target/debug/ap-run-eval"` — read at t05:77, t07:117,
    t08:125, t10:77, t11:108-109. ap-run-eval accepts exactly the `--tier 0`
    ap-conform appends (crates/ap-cli/src/bin/ap_run_eval.rs:4-25).
  - `APX_CLI="target/debug/antipythany run"` — read at t09:158, t14:80,
    t15:63, t16:126. NOTE t09/t14/t15/t16 invoke `_cli_cmd() + ["build",...]`
    / `["--version"]` / `["new",...]` verbs; with APX_CLI ending in `run`
    those become `<antipythany run build ...>` — wrong argv shape for those
    suites, but they are all xfail(strict=False) awaiting 02.06/04.02, so
    CI stays green; master.md:37 mandates this exact value, so I kept it
    (gap noted below for when those xfails flip).
  - `AP_CONFORM_REFERENCE_PYTHON` exported via GITHUB_ENV (same step shape
    as the `rust` job) — honored by pytest t06/t07/t08/t09/t11/t16/
    fstrings_corpus REF_PY reads.
  - `AP_REQUIRE_REFERENCE=1` already workflow-level (00.01) — now genuinely
    enforced by 00.02's committed common/mod.rs (Rust suites; this job's
    pytest track doesn't read it — gap below).
- **Compat symlink step:** t05_values:57 and t10_threading:73 STILL hardcode
  REF_PY="/opt/homebrew/bin/python3.14" (00.02 converted only the Rust
  side — its PROGRESS gap #1 explicitly defers this to 00.03). On ubuntu
  that path is absent -> FileNotFoundError red. Step symlinks
  /opt/homebrew/bin/python3.14 -> the setup-python 3.14 binary (sudo on
  linux, plain mkdir on macos; no-op when it already exists). No gate
  weakened, same interpreter; step is commented "DELETE when t05/t10 honor
  the env var".
- **Job summary nicety (kept, not fiddly):** report-only `if: always()`
  step runs `ap-conform run --corpus tests/corpus/<fam>/*.py --impl
  <reference> --json` per 8 families, jq-formats summary counts into
  $GITHUB_STEP_SUMMARY. Exit 0 always; per-family failures print a row
  saying so.
- `timeout-minutes: 180` (local full pytest track ~1h); `fail-fast: false`
  so ubuntu red doesn't cancel the macOS run on main.

### Local verification (this box: Homebrew rustc 1.94.0 + python 3.14.7;
### working tree has uncommitted sibling 02.07 threading src, binaries
### already built in target/debug)
- YAML parse: PyYAML 6.0.3 `safe_load` on the full file — job graph,
  needs, matrix, outputs, step env all structurally as intended.
- Every multi-line `run:` block passes `bash -n`.
- Axis script simulated for push/main, pull_request, push/feature: emits
  exactly the two expected JSON arrays; `json.loads` (fromJSON-equivalent)
  validates both.
- ap-conform --json + the jq row template executed locally against the
  real binary: rc 0, correct `| family | total | ... |` markdown row.
- Subset pytest with the job's env vars (APX_IMPL/APX_CLI/
  AP_CONFORM_REFERENCE_PYTHON/AP_REQUIRE_REFERENCE): `pytest tests/t05_values
  tests/t11_stdlib -q` -> 476 passed, 2 skipped, 80 xfailed, 69 xpassed,
  rc 0, 34s. Full `pytest tests/` NOT run here: sibling wave-A full run
  (PID 3750, output /tmp/ap_pytest_waveA.txt, ~23% at check time, 0 F/0 E)
  is already occupying the suite — competing full run deliberately avoided
  per brief.
- actionlint not installed locally (no installs permitted) — structural
  YAML + bash -n used instead.

### HONEST gaps
1. No remote green run possible from this leaf (no commits/pushes permitted;
   the leaf's own gate "workflow green on both OSes" is unverifiable here —
   orchestrator must push a branch and `gh run watch`).
2. macOS matrix leg entirely unverified locally (can't emulate macos-14
   runner: rust-cache on macOS, sudo-free symlink path assumed writable —
   GH macos runners run as admin, /opt/homebrew exists via brew prefix
   but bin/python3.14 may not; mkdir -p /opt/homebrew/bin as the runner
   user should work, unproven).
3. t09/t14/t15/t16 suites append verbs to APX_CLI ("build", "new",
   "--version"), which the brief's mandated `...antipythany run` value
   breaks — they stay green only because every one is xfail(strict=False)
   today. When 02.06/04.02 flip them, the workflow needs an APX_CLI split
   (driver without `run`). Flagged in a workflow comment.
4. ap-parse `ap-parse-check` bin does not exist in this tree (t01_parse
   suite handles it via its _MissingBin stub; no workflow env needed).
5. The compat symlink masks (does not fix) the t05/t10 Homebrew hardcodes;
   real fix is env-var conversion in tests/*.py, deferred by 00.02 to a
   later leaf/branch (01 test-integrity).
6. Job-summary step re-runs ~270 corpus cases through the reference (~1-2
   min); acceptable, it is report-only and runs even when pytest failed.
7. `gh` remote validation (Actions syntax tab / actionlint CI) not run.

---

## wave-A-gap followup — ap-cli + ap-obj conversion (2026-10-20, subagent)

Closes 00.02 HONEST gaps #2 and #3: the last two inconsistent Rust
differential suites now use the shared reference-discovery seam, and
ap-conform's finder honors `AP_CONFORM_REFERENCE_PYTHON`.

### Files converted
- `crates/ap-cli/tests/common/mod.rs` (NEW) + `crates/ap-obj/tests/common/mod.rs`
  (NEW) — verbatim copies of `crates/ap-run/tests/common/mod.rs` (compiles
  clean per-crate; `#![allow(dead_code)]` sufficient, no adjustment needed).
- `crates/ap-cli/tests/stdlib_modules_differential.rs` — homebrew+PATH ladder
  replaced by `mod common;` + `common::reference_or_skip()` in
  `assert_differential` (early return on None; all 19 probe cases byte-for-byte
  unchanged). No Cargo.toml changes.
- `crates/ap-obj/tests/cpython_differential.rs` — spawn-only probe
  (no version check) replaced by the version-checked seam: `diff!` macro
  gate, the 3 standalone `#[test]` gates, and `cpython()` all go through
  `common::reference_or_skip()`. All probe cases unchanged. No Cargo.toml
  changes.
- `crates/ap-conform/src/lib.rs` — `find_reference_interpreter()` gains
  step 0: `AP_CONFORM_REFERENCE_PYTHON` (trim, non-empty, version-checked via
  the existing `is_python_314`), then keeps its own `python3.14` -> `python3`
  candidates unchanged. 14-line diff; behavior identical when the env is
  unset. ap-conform was clean in git status (no sibling conflict). No unit
  tests in ap-conform/src assert candidate lists; `tests/harness_selfcheck.rs`
  only re-verifies the returned interpreter reports 3.14 — still green.

### Gates executed (fresh target/debug binaries; PATH=scrub via env -i
PATH=/usr/bin:/bin so /opt/homebrew is gone; /usr/bin/python3 = 3.9.6 present)
1. `cargo test -p ap-cli` normal env: 19 passed / 0 failed.
   `cargo test -p ap-obj` normal env: 25 passed / 0 failed (suite incl.
   pinned unit tests). `cargo test -p ap-conform`: 3 passed / 0 failed.
2. require mode: scrubbed PATH + `AP_REQUIRE_REFERENCE=1` ->
   stdlib_modules_differential exit 101, 19x canonical "reference interpreter
   not found (AP_REQUIRE_REFERENCE=1)"; cpython_differential exit 101,
   25x same message.
3. skip mode: scrubbed PATH, no flag -> rc 0; `--nocapture` shows 19 / 25
   "SKIP: python3.14 not found" lines.
4. ladder step 1: scrubbed PATH +
   `AP_CONFORM_REFERENCE_PYTHON=/opt/homebrew/bin/python3.14` -> 19/19 + run
   for real (0 SKIP lines).
5. version rejection: env var -> /usr/bin/python3 (3.9.6) + REQUIRE=1 ->
   warning "does not report Python 3.14" + hard fail (never diffs vs wrong
   version). Same semantics in ap-conform: bad env value falls through to
   `python3.14` candidate.
6. ap-conform self-check CLI: `cargo run -p ap-conform -- run --corpus
   'tests/corpus/values/V001*.py' --impl 'target/debug/ap-run-eval'` -> PASS
   (values corpus uses V0NN caps, not v001 — globbed what exists). With the
   env var set, the banner reports the override path.
7. `rustfmt --edition 2021` on all 5 touched .rs files (already formatted;
   --check clean). `cargo build --workspace` exit 0 (sibling ap-run src WIP
   compiles with warnings only).

### Notes / gaps
- Probe case contents untouched byte-for-byte in both suites; only the
  discovery mechanism changed, per brief.
- Cargo.toml files NOT touched (ownership constraint; nothing needed).
- Skipped-mode "green" stays honest-but-empty by design (00.02 gap #4); the
  CI-enforcing mode is AP_REQUIRE_REFERENCE=1 as set by 00.01/00.03 jobs.
- Nothing committed (per brief).
