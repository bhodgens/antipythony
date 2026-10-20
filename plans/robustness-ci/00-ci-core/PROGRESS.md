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
