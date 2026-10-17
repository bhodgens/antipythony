# T-15 — release/install (gates implementation leaf 04.03 Release engineering)

Authorship: written from the plan/contract surfaces ONLY
(`plans/antipythany/04-packaging-ecosystem/orchestrator.md` 04.03 + the Plan 04
CLI verb table, `plans/antipythany/master.md` §License and attribution contract,
`plans/antipythany/00-foundation-contract/orchestrator.md` 00.02,
`docs/attribution.md` (committed notices command-surface doc),
`docs/compat-contract.md` C-26 byte-anchor spirit, `plans/antipythany/tests/README.md`).
No implementation source under `crates/` was read. Suite:
`tests/t15_release/test_t15_release.py`.

## Purpose

Gate leaf 04.03: **release engineering** — the artifacts a fresh user touches.
04.03 text (verbatim): "CI matrix (linux-musl static, linux-gnu, macos-arm64,
macos-x86_64), release artifacts + checksums, THIRD_PARTY_NOTICES embedded in
binaries and emitted by `--version --notices`, install script. Gate:
fresh-machine install -> new -> add rich -> run -> build, scripted, green."

Unlike T-09 (whose binary is locally buildable), most of T-15's surface is
**CI-produced release output that cannot exist until 04.03 ships**. The honest
core is therefore split:

1. **Green-now static assertions over repo state** — facts that MUST be true
   at gate time and are checkable today without any release artifact:
   the LICENSE carries Apache-2.0 + LLVM-exception text (master.md License
   table, 00.01); `THIRD_PARTY_NOTICES` exists, satisfies the documented
   determinism invariant (entries sorted by (crate name, version) —
   docs/attribution.md, the property that makes "byte-identical regeneration"
   meaningful), lists the five ruff donor crates (master.md attribution
   contract + 00.02 end-to-end proof), documents the xtask generation/`--check`
   command surface, and carries no Nuitka entry (attribution.md: its presence
   "is a bug").
2. **Artifact-gated xfails** — the release engineering itself: notices
   freshness (`--check` green), license-policy conformance of the notices file
   against deny.toml, a CI runner config naming matrix targets, per-target
   release artifacts with verifying `.sha256` checksums, a POSIX-parseable
   install script, `--version --notices` emitting bytes equal to the committed
   notices file, and the fresh-machine scripted gate as ONE end-to-end test.

## Testability contract under test

- Expose (04.03, verbatim): "CI matrix (linux-musl static, linux-gnu,
  macos-arm64, macos-x86_64), release artifacts + checksums,
  THIRD_PARTY_NOTICES embedded in binaries and emitted by `--version
  --notices`, install script."
- Gate (04.03, verbatim): "fresh-machine install -> new -> add rich -> run ->
  build, scripted, green." The five verbs are exactly the Plan 04 CLI surface
  (`antipythany new myapp` / `add <pkg>` / `run app.py` / `build app.py` ->
  `./app`).
- Notices gate surface (00.02 Expose + docs/attribution.md): `cargo run -p
  xtask -- notices` regenerates deterministically (sorted by (crate, version),
  fixed template); `cargo run -p xtask -- notices --check` exits 1 on drift
  with `NOTICES DRIFT`. master.md: "release build fails if the file is stale"
  — so freshness is a 04.03 precondition even though the generator is 00.02's.
- License policy (master.md §License and attribution contract + docs/
  attribution.md Allowed list, mirrored in `deny.toml`): MIT / Apache-2.0 /
  Apache-2.0 WITH LLVM-exception / BSD-3-Clause / BSD-2-Clause / ISC / Zlib /
  Unicode-3.0 / CDLA-Permissive-2.0; GPL/AGPL/LGPL/MPL denied.
- C-26 relevance (byte-anchored stable outputs): the notices-embedding test
  byte-compares emitted `--version --notices` output against the committed
  file — the same stable-vs-unstable discipline: what is contractually stable
  gets byte-anchored, and only the trailing-newline edge is left flexible
  because nothing pins it (G-2).
- CLI resolution mirrors T-09: env `APX_CLI` (whitespace-split) →
  `antipythany` on PATH → `target/{release,debug}/antipythany`.
  Artifact dir: env `APX_DIST_DIR` (default `repo_root/dist`); install script:
  env `APX_INSTALL_SH`; release binary for the embedding test:
  env `APX_DIST_BIN`; offline `add` stand-in: env `APX_ADD_SPEC`.

### Which checks are green today vs artifact-gated-xfail

**GREEN today (9):** LICENSE Apache-2.0 text; LICENSE LLVM-exception block
(title + substance: "embedded into an Object form", "Sections 4(a), 4(b) and
4(d)"); two byte-level marker checks (parametrized ×2); THIRD_PARTY_NOTICES
exists and non-trivial; entries sorted by (crate, version) (120 entries
verified); the five ruff crates present; generation-command + `--check`
surface documented in the file header (xtask a workspace member); no `nuitka`
in Cargo.lock or THIRD_PARTY_NOTICES.

**XFAIL — artifact-gated (10):** notices `--check` green (OBSERVED: exits 1
with `NOTICES DRIFT` — the committed file IS stale at authoring); license
policy conformance (OBSERVED violation: `unicode_names2` AND-branch
`Unicode-DFS-2016` unsatisfiable under the allow list — G-5); CI runner config
exists naming a matrix target (OBSERVED: no `.github/`, no justfile/Makefile);
per-target artifact+checksum tests ×4 (linux-musl / linux-gnu / macos-arm64 /
macos-x86_64; OBSERVED: no dist dir at all); install script `sh -n` parseable
(OBSERVED: no install script anywhere); `--version --notices` embedding
(OBSERVED: scaffold CLI prints nothing for `--version`, silently exits 0);
fresh-machine scripted gate (OBSERVED: stub verbs produce no project dir).
The policy-conformance test is deliberately an in-test assertion rather than a
skip so drift fails loudly under 04.03's reason string (cargo-deny is not
installed here — G-4 — so a pure-stdlib classifier stands in; replacing it
with `cargo deny check licenses` later only makes the check stronger).

## Artifact naming scheme expectations (transcribed from 04.03 + gaps)

04.03 pins the four matrix target ids verbatim: `linux-musl static, linux-gnu,
macos-arm64, macos-x86_64`. Everything downstream is unpinned; the suite's
workable readings and their gap ids:

| Surface | Pinned by 04.03? | T-15's working assumption | Gap |
| --- | --- | --- | --- |
| Artifact filenames | NO — only "release artifacts" | any file in the dist dir whose name contains the target id (case-insensitive) | G-1 |
| Checksum format | NO — only "+ checksums" | companion `<artifact>.sha256`, GNU (`<64hex>  <file>`) or BSD (`SHA256 (<file>) = <64hex>`) first line; the hash MUST verify against artifact bytes | G-2 |
| Dist/artifact directory | NO | `repo_root/dist`, override `APX_DIST_DIR` | G-1 |
| Install-script path/URL | NO — only "install script" | committed `dist/install.sh` / `scripts/install.sh`, override `APX_INSTALL_SH`; `sh -n` clean; mentions `antipythany` + a sha256/checksum hook | G-1/G-2 |
| CI runner platform | NO | any committed CI config (.github/workflows, .gitlab-ci, circleci) or justfile/Makefile referencing ≥1 target id | G-3 |
| `--version --notices` output | flag pinned; exact bytes policy NOT | emitted bytes == committed file bytes modulo trailing newline | G-2 |
| Scaffolded entry file after `new` | owned by 04.01, not 04.03 | glob the first `.py` in the project dir | G-6 |

## Fresh-machine scripted gate — step table

One xfail test (`test_fresh_machine_scripted_gate`) executing 04.03's gate
verbatim, scripted inside the test (no CI needed to run it once artifacts
land):

| # | Step | Command (Plan 04 surface) | Assertion |
| --- | --- | --- | --- |
| 1 | install | resolved CLI under stripped env (fresh `HOME=tmpdir`) | `--version` rc 0 |
| 2 | new | `antipythany new myapp` | rc 0 AND `myapp/` dir created |
| 3 | add rich | `antipythany add rich` (offline override `APX_ADD_SPEC`) | rc 0 AND a lock file recorded |
| 4 | run | `antipythany run <entry.py>` | rc 0 AND non-empty stdout |
| 5 | build | `antipythany build <entry.py>` | rc 0 AND `./app` exists, executable, standalone run rc 0 with stdout |

"Fresh machine" is approximated (temp HOME, fresh cwd, stripped env); true
VM-level freshness is a CI concern — the runner image is unpinned (G-3).
`add rich` is network-dependent (uv → PyPI, per Plan 04 dependency model); no
local pure-wheel fixture exists at authoring (`tests/fixtures/` holds only
parse-fixture `.py` files), so an `APX_ADD_SPEC` override names a local wheel
spec as the offline stand-in — the cheap stand-in path, else G-6 gap.

## CONTRACT-GAPS (findings to file back per README §Protocol 5)

- **G-1 — artifact filenames and dist layout unpinned.** 04.03 says "release
  artifacts" with no naming scheme, no directory, no archive-vs-raw-binary
  decision, and no install-script URL/path. T-15 scans by target-id substring
  and accepts `dist/` / `scripts/` locations with env overrides. When 04.03
  ships, pin the scheme in the leaf text and these tests get a rename check
  (STRONGER only — C-33).
- **G-2 — checksum format and trailing-newline policy unpinned.** "+
  checksums" names no algorithm or file layout; T-15 assumes sha256
  sidecar files in GNU/BSD `sha256sum` line forms (sha256 is the only hash
  any read surface mentions: nowhere — it is our assumption). Similarly the
  byte-equality policy for `--version --notices` output (exact vs newline-
  trimmed) is unpinned; T-15 trims trailing newlines on both sides.
- **G-3 — matrix runner platform/config unpinned.** 04.03 names four targets
  but no runner (GitHub Actions? self-hosted musl image?) and no file format.
  T-15 accepts any committed CI config or justfile that references a target
  id. "linux-musl static" additionally implies the musl binary is what the
  15MB bound (T-09 G-2) applies to — cross-ref, not re-gated here.
- **G-4 — cargo-deny not installed on the authoring host.** The master.md CI
  gate names `cargo-deny` + `cargo-about`; neither drives the local pipeline
  (attribution.md says cargo-about was passed over; `cargo deny` is missing).
  T-15's policy test is a pure-stdlib classifier over the committed notices +
  deny.toml allow list — an approximation of cargo-deny's AND/OR semantics,
  to be replaced by the real tool in CI (test stays, only the classifier is
  strengthened).
- **G-5 — OBSERVED policy violation: `unicode_names2` is
  `(MIT OR Apache-2.0) AND Unicode-DFS-2016`; `Unicode-DFS-2016` appears in
  neither the attribution.md Allowed list nor a deny.toml exception.** Either
  add it to the allow list with a comment naming the leaf that justified it
  (docs/attribution.md §License policy prescribes exactly that mechanism) or
  the dependency must change. This must be resolved before a release ships
  these notices — filed against 00.02 policy + 04.03 (which embeds the file).
- **G-6 — `new`'s scaffolded file names and `add`'s offline story are 04.01
  surface, unnamed in 04.03.** 04.01's gate says the "new/add/run flow with one
  pure-Python dep (e.g. rich) works end to end" but never fixes the scaffolded
  entry filename or a lock filename pattern; T-15 globs. And no local pure-
  wheel fixture exists, so the scripted gate's `add rich` is network-bound;
  `APX_ADD_SPEC` is the offline seam. If a vendored tiny wheel fixture is
  added later, point the env at it in CI.
- **G-7 — OBSERVED staleness: `notices --check` currently exits 1
  (`NOTICES DRIFT`).** The committed THIRD_PARTY_NOTICES is stale against
  Cargo.lock at authoring time (00.02 drift gate working as designed). T-15
  xfails on it rather than silently regenerating — regeneration is
  implementation/owner action (`cargo run -p xtask -- notices`), not a test
  track action.

## Per-test citations (suite `tests/t15_release/test_t15_release.py`)

| Test | Assertion | Cites | Status today |
| --- | --- | --- | --- |
| `test_license_file_is_apache2` | LICENSE has Apache-2.0 header | master.md License table; 00.01 | GREEN |
| `test_license_contains_llvm_exception` | LLVM-exception block incl. substance | 00.01 (spdx.org LLVM-exception copy); master.md rationale | GREEN |
| `test_license_marker_strings_exact` ×2 | byte-level presence of both block markers | C-26 stable-anchor discipline | GREEN |
| `test_notices_file_exists` | committed, non-trivial | 00.02; master.md attribution contract | GREEN |
| `test_notices_entries_sorted_by_name_version` | entries == sorted by (crate, version) | 00.02 Expose determinism; attribution.md | GREEN |
| `test_notices_lists_ruff_donor_crates` ×5 crates in one test | all five ruff component crates present | master.md attribution; 00.02; attribution.md Known-good | GREEN |
| `test_notices_documents_generation_command_surface` | header names `xtask ... notices [--check]`; xtask is a member | attribution.md command surface; 00.02 | GREEN |
| `test_notices_carries_no_nuitka` | no nuitka in notices or Cargo.lock | attribution.md Nuitka note ("is a bug") | GREEN |
| `test_notices_check_gate_green` | `cargo run -p xtask -- notices --check` rc 0 | 00.02 Gate; master.md "release build fails if stale" | XFAIL — OBSERVED drift rc 1 (G-7) |
| `test_notices_license_policy_conformance` | every entry expr satisfiable by deny.toml allow | master.md CI gate; attribution.md Allowed list | XFAIL — OBSERVED unicode_names2 (G-5) |
| `test_ci_matrix_runner_config_exists` | some CI config references a matrix target id | 04.03 CI matrix; G-3 | XFAIL — none exists |
| `test_release_artifact_per_matrix_target` ×4 | per target: artifact + `.sha256` whose hash verifies bytes | 04.03 "release artifacts + checksums"; G-1/G-2 | XFAIL — no dist dir |
| `test_install_script_exists_and_posix_parseable` | `sh -n` clean; names antipythany; checksum hook | 04.03 "install script"; G-1/G-2 | XFAIL — none exists |
| `test_version_notices_embedding_matches_committed_file` | emitted bytes == committed bytes (trailing-NL relaxed) | 04.03 verbatim `--version --notices`; C-26 byte-anchor | XFAIL — stub prints nothing |
| `test_fresh_machine_scripted_gate` | 5-step table: install→new→add rich→run→build→standalone run | 04.03 Gate verbatim; Plan 04 verb table; 04.01 gate (rich) | XFAIL — stub verbs |

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t15_release -q
```

From the repo root. Today: **9 passed, 10 xfailed, 0 errors** (19 total).
Overrides: `APX_CLI`, `APX_DIST_DIR`, `APX_INSTALL_SH`, `APX_DIST_BIN`,
`APX_ADD_SPEC`.

## xfail policy

- All 04.03-side tests are `xfail(strict=False)`: release engineering does not
  exist yet (CI config absent, stub CLI), so bodies fail inside the envelope.
  The two OBSERVED-drift xfails (G-5, G-7) are genuine repo-state findings the
  04.03 owner must clear, not hypotheticals.
- After 04.03 lands: XPASS on the embedding/artifact tests means the release
  is shippable; any FAIL is a gate failure — fix the implementation, never the
  test (master.md protocol; C-33).
- The green-now half must STAY green: a break in LICENSE/Notices-structure
  checks means the attribution contract regressed, independent of 04.03.

## Protocol notes

- Tests commit BEFORE 04.03 completes (README §Protocol 3); the gate runner is
  an agent that wrote neither these tests nor the release tooling (§Protocol 4).
- No file outside `tests/t15_release/` and this doc was written. `deny.toml`,
  `THIRD_PARTY_NOTICES`, `Cargo.lock` were read only.
