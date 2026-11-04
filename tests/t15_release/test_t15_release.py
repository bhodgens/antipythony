# T-15 — release/install (gates implementation leaf 04.03 Release engineering).
#
# Authorship: written from the spec surfaces ONLY
# (plans/antipythony/04-packaging-ecosystem/orchestrator.md 04.03 + CLI verb
# table, plans/antipythony/master.md license & attribution contract,
# plans/antipythony/00-foundation-contract/orchestrator.md 00.02,
# docs/attribution.md notices command surface, docs/compat-contract.md C-26,
# plans/antipythony/tests/README.md). No implementation source under crates/
# was read. Per-test citations and CONTRACT-GAPS:
# plans/antipythony/tests/T-15-release-install.md.
#
# Structure:
#   GREEN NOW   — repo-state static assertions that MUST hold at 04.03 gate
#                 time and are already checkable: LICENSE carries the
#                 Apache-2.0 + LLVM-exception text (master.md License table +
#                 00.01); THIRD_PARTY_NOTICES exists, is entry-sorted (the
#                 documented determinism property, 00.02), lists the ruff
#                 donor crates, and documents the xtask notices surface
#                 (docs/attribution.md).
#   XFAIL (04.03) — everything that is a release ENGINEERING artifact: the
#                 notices --check drift gate (currently DRIFTS, observed at
#                 authoring), license-policy conformance of the notices file
#                 to the allow list (unicode_names2 Unicode-DFS-2016 today),
#                 CI runner config existence, per-target artifact + .sha256
#                 checksum verification, POSIX-parseable install script,
#                 `--version --notices` embedding == committed bytes, and the
#                 fresh-machine scripted gate (new -> add rich -> run -> build).

import hashlib
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
LICENSE = REPO_ROOT / "LICENSE"
NOTICES = REPO_ROOT / "THIRD_PARTY_NOTICES"
DENY = REPO_ROOT / "deny.toml"
ATTRIBUTION_DOC = REPO_ROOT / "docs" / "attribution.md"

# 04.03 text verbatim: "CI matrix (linux-musl static, linux-gnu, macos-arm64,
# macos-x86_64)". Exact artifact filenames are NOT pinned by any spec surface —
# G-1. We transcribe the four target names here and reference them; artifact
# tests scan for files whose name contains the target id (case-insensitive).
MATRIX_TARGETS = ["linux-musl", "linux-gnu", "macos-arm64", "macos-x86_64"]

XFAIL_0403 = pytest.mark.xfail(
    strict=False,
    reason="awaits 04.03 release engineering (CI artifacts / checksums / "
    "install script / embedded notices / fresh-machine gate)",
)


def _cli_cmd() -> list:
    """The `antipythony` driver (Plan 04 CLI surface). Resolution mirrors
    T-09: env APX_CLI (whitespace-split) -> PATH -> target/{debug,release}."""
    override = os.environ.get("APX_CLI")
    if override:
        return shlex.split(override)
    found = shutil.which("antipythony")
    if found:
        return [found]
    for profile in ("release", "debug"):
        cand = REPO_ROOT / "target" / profile / "antipythony"
        if cand.exists():
            return [str(cand)]
    raise FileNotFoundError(
        "antipythony CLI not found (set APX_CLI=<cmdline> to override)"
    )


def _notices_entries(text: str):
    """Parse the documented notices shape (docs/attribution.md: 'entries are
    sorted by (crate name, version)', fixed template). Entry header block:
    ===...\n<name> v<version>\n---...\nLicense: <expr>."""
    return re.findall(r"^=+\n(\S+) v(\S+)\n-+\nLicense: (.*)$", text, re.M)


def _spdx_or_groups(expr: str):
    """Split an SPDX-ish expression on top-level ' AND ', paren-aware."""
    parts, depth, cur = [], 0, ""
    for ch in expr:
        if ch == "(":
            depth += 1
            cur += ch
        elif ch == ")":
            depth -= 1
            cur += ch
        elif depth == 0 and cur.endswith(" AND"):
            parts.append(cur[:-4].strip())
            cur = ""
        else:
            cur += ch
    parts.append(cur.strip())
    return parts


def _spdx_legs(group: str):
    """The OR-legs of one top-level AND group (WITH-exception kept attached
    to its base id, and the bare base id also offered)."""
    ids = set()
    for leg in re.split(r"\s+OR\s+", group.strip()):
        leg = leg.strip().strip("()")
        if " WITH " in leg:
            ids.add(leg)
            ids.add(leg.split(" WITH ")[0].strip())
        elif leg:
            ids.add(leg)
    return ids


def _expr_satisfiable(expr: str, allowed: set):
    """cargo-deny semantics: EVERY top-level AND group must be satisfiable by
    at least one allowed id (an OR group picks a license; an AND branch with no
    allowed option is a violation). Returns the offending AND group or None."""
    for grp in _spdx_or_groups(expr):
        if not (_spdx_legs(grp) & allowed):
            return grp
    return None


def _deny_allowed() -> set:
    """Read the allow list out of deny.toml (a committed config, readable per
    README inputs; 00.02 defines its policy)."""
    text = DENY.read_text()
    block = text.split("allow = [", 1)[1].split("]", 1)[0]
    return set(re.findall(r'"([^"]+)"', block))


# ---------------------------------------------------------------------------
# HALF 1 — GREEN NOW: repo-state static assertions (must hold at gate time).
# ---------------------------------------------------------------------------


def test_license_file_is_apache2():
    """master.md License table: 'Apache-2.0 WITH LLVM-exception (whole
    project)'; 00.01 Expose: LICENSE = Apache-2.0 + LLVM-exception text.
    Checks the canonical Apache-2.0 header is present."""
    assert LICENSE.exists(), "LICENSE missing (00.01)"
    text = LICENSE.read_text()
    assert "Apache License" in text and "Version 2.0" in text, (
        "LICENSE lacks the Apache-2.0 text"
    )


def test_license_contains_llvm_exception():
    """00.01 says to copy https://spdx.org/licenses/LLVM-exception.html.
    The exception block must be present in the same LICENSE file — the
    exception is what lets user binaries embed our runtime without carrying
    our NOTICE file (master.md rationale)."""
    text = LICENSE.read_text()
    assert "LLVM Exceptions to the Apache 2.0 License" in text, (
        "LLVM-exception block missing from LICENSE"
    )
    # substance of the exception, not just its title:
    assert "embedded into an Object form" in text
    assert "Sections 4(a), 4(b) and 4(d)" in text


@pytest.mark.parametrize(
    "marker", ["Apache License", "LLVM Exceptions to the Apache 2.0 License"]
)
def test_license_marker_strings_exact(marker):
    """Byte-level guard on the two required license blocks (C-26's spirit:
    stable anchors are byte-checked; the exception grant is contractual)."""
    raw = LICENSE.read_bytes()
    assert marker.encode() in raw


def test_notices_file_exists():
    """master.md §License and attribution contract + 00.02: THIRD_PARTY_NOTICES
    is a committed artifact of the attribution gate."""
    assert NOTICES.exists(), "THIRD_PARTY_NOTICES missing (00.02)"
    assert NOTICES.stat().st_size > 1000, "THIRD_PARTY_NOTICES suspiciously empty"


def test_notices_entries_sorted_by_name_version():
    """00.02 Expose (verbatim): 'xtask notices regenerates
    THIRD_PARTY_NOTICES deterministically'; docs/attribution.md pins HOW:
    'entries are sorted by (crate name, version)'. The committed file must
    satisfy that invariant — it is checkable today without any release
    artifact, and it is the property that makes 'byte-identical regeneration'
    meaningful."""
    entries = _notices_entries(NOTICES.read_text())
    assert len(entries) > 0, "no notices entries parsed — template drift (G-3)"
    keys = [(name, ver) for name, ver, _lic in entries]
    assert keys == sorted(keys), (
        f"notices entries not sorted by (crate, version): first offender at "
        f"{next(i for i in range(1, len(keys)) if keys[i] < keys[i-1])}"
    )


def test_notices_lists_ruff_donor_crates():
    """master.md attribution contract lists ruff parser (MIT) with 'notice in
    THIRD_PARTY_NOTICES'; 00.02 Expose adds ruff to ap-parse 'to prove the
    pipeline works end to end'; docs/attribution.md §Known-good names the five
    ruff component crates. Their presence is the pipeline end-to-end proof."""
    text = NOTICES.read_text()
    for crate in (
        "ruff_python_parser",
        "ruff_python_ast",
        "ruff_text_size",
        "ruff_python_trivia",
        "ruff_source_file",
    ):
        assert re.search(rf"^=+\n{crate} v", text, re.M), (
            f"{crate} missing from THIRD_PARTY_NOTICES (00.02 / attribution.md)"
        )


def test_notices_documents_generation_command_surface():
    """docs/attribution.md is the committed command surface: the file header
    must name the generation command `cargo run -p xtask -- notices` and the
    CI gate `... --check` (00.02 Gate: stale notices must fail)."""
    text = NOTICES.read_text()
    assert "cargo run -p xtask -- notices" in text, (
        "notices header lacks the documented generation command"
    )
    assert "notices --check" in text, (
        "notices header lacks the documented --check CI gate"
    )
    # the generator must be a workspace member (00.02: CI recipe via xtask)
    members = (REPO_ROOT / "Cargo.toml").read_text()
    assert "crates/xtask" in members, "xtask not a workspace member"


def test_notices_carries_no_nuitka():
    """docs/attribution.md Nuitka note + master.md: Nuitka is AGPL, subprocess
    tool only — 'a nuitka entry anywhere in the Rust dependency graph is a
    bug'. Asserted over the two committed graphs this test may read
    (Cargo.lock, THIRD_PARTY_NOTICES)."""
    for path in (REPO_ROOT / "Cargo.lock", NOTICES):
        assert "nuitka" not in path.read_text().lower(), (
            f"nuitka appears in {path.name} — attribution.md declares this a bug"
        )


# ---------------------------------------------------------------------------
# HALF 2 — 04.03 GATE (xfail, strict=False). Release engineering artifacts.
# ---------------------------------------------------------------------------


def test_notices_check_gate_green():
    """04.03: THIRD_PARTY_NOTICES embedded in binaries must equal the
    committed file, so the committed file must be FRESH first. Documented
    gate (docs/attribution.md, 00.02): `cargo run -p xtask -- notices --check`
    exits 0, fails with 'NOTICES DRIFT' otherwise. The drift observed at
    authoring was fixed 2026-10-17 (bughunt wave 1: xtask static_entries +
    regen) and is now CI-enforced (.github/workflows/attribution.yml), so
    this gate test is live — a stale notices file must fail it."""
    proc = subprocess.run(
        ["cargo", "run", "-p", "xtask", "--quiet", "--", "notices", "--check"],
        capture_output=True,
        cwd=REPO_ROOT,
        timeout=300,
    )
    out = (proc.stdout + proc.stderr).decode(errors="replace")
    assert proc.returncode == 0, f"notices --check rc={proc.returncode}:\n{out}"


@XFAIL_0403
def test_notices_license_policy_conformance():
    """master.md CI gate: cargo-deny policy over the dependency graph;
    docs/attribution.md pins the Allowed list. Release builds ship these
    notices, so every entry's expression must be satisfiable by deny.toml's
    allow list. Pure-stdlib classifier (cargo-deny not installed on this host
    at authoring — G-4), run as an assertion so it xfails rather than
    skipping. OBSERVED AT AUTHORING: unicode_names2 carries
    '(MIT OR Apache-2.0) AND Unicode-DFS-2016'; the AND branch
    Unicode-DFS-2016 is NOT in the allow list nor a documented exception, so
    the expression is unsatisfiable — drift to fix or to document in
    deny.toml [[licenses.exceptions]] before release (G-5)."""
    allowed = _deny_allowed()
    violations = []
    for name, ver, lic in _notices_entries(NOTICES.read_text()):
        offender = _expr_satisfiable(lic, allowed)
        if offender is not None:
            violations.append((name, lic, offender))
    assert not violations, (
        f"notices entries with an AND group outside deny.toml allow list: "
        f"{violations}"
    )


@XFAIL_0403
def test_ci_matrix_runner_config_exists():
    """04.03 text: 'CI matrix (linux-musl static, linux-gnu, macos-arm64,
    macos-x86_64)'. A CI runner config declaring those targets must exist in
    the repo. Runner platform is entirely unpinned (G-3): we only require SOME
    committed CI config (GitHub Actions workflows, GitLab CI, or a justfile /
    Makefile recipe) that references at least one matrix target id. Today:
    no .github/, no justfile — this fails."""
    candidates = []
    for d in (".github/workflows", ".gitlab-ci.yml", ".circleci"):
        p = REPO_ROOT / d
        if p.exists():
            candidates.extend(sorted(p.rglob("*.yml")) + sorted(p.rglob("*.yaml")))
    for f in ("justfile", "Justfile", "Makefile"):
        p = REPO_ROOT / f
        if p.exists():
            candidates.append(p)
    assert candidates, "no CI/runner config found in repo (04.03 CI matrix)"
    blob = "\n".join(p.read_text(errors="replace") for p in candidates).lower()
    hits = [t for t in MATRIX_TARGETS if t.replace("_", "-") in blob or t in blob]
    assert hits, f"CI config(s) {candidates} reference none of {MATRIX_TARGETS}"


def _dist_dir() -> Path:
    """Release artifact directory. NAME UNPINNED (G-1): default
    repo_root/dist; override with APX_DIST_DIR."""
    override = os.environ.get("APX_DIST_DIR")
    return Path(override) if override else REPO_ROOT / "dist"


def _artifacts_for(target: str) -> list:
    """Scan the (unpinned) dist dir for artifacts referencing a matrix target.
    Artifact FILENAMES are unpinned by any spec surface (G-1) — the honest
    reading of 04.03 'release artifacts + checksums' is: per-target release
    bundle + a checksum file naming it. We glob any file whose name contains
    the target id."""
    d = _dist_dir()
    if not d.exists():
        return []
    return sorted(
        p
        for p in d.iterdir()
        if p.is_file()
        and target.lower() in p.name.lower()
        and not p.name.endswith(".sha256")
    )


@pytest.mark.parametrize("target", MATRIX_TARGETS)
@XFAIL_0403
def test_release_artifact_per_matrix_target(target):
    """04.03: 'CI matrix (linux-musl static, linux-gnu, macos-arm64,
    macos-x86_64), release artifacts + checksums'. Each matrix row must yield
    at least one non-checksum artifact whose name carries the target id, plus
    a companion .sha256 file (the checksum FORMAT is unpinned — G-2; we accept
    GNU '<hash>  <file>' and BSD 'SHA256 (<file>) = <hash>' line shapes and
    REQUIRE the hash to verify against the artifact bytes)."""
    arts = _artifacts_for(target)
    assert arts, (
        f"no release artifact for matrix target {target!r} in {_dist_dir()} "
        f"(artifact naming unpinned — G-1)"
    )
    d = _dist_dir()
    for art in arts:
        ck = d / (art.name + ".sha256")
        assert ck.exists(), f"missing checksum file {ck.name} for {art.name}"
        text = ck.read_text()
        m = re.match(
            r"^([0-9a-fA-F]{64})(?:  .*)?$", text.splitlines()[0].strip()
        ) or re.match(
            r"^SHA256 \((.+?)\) = ([0-9a-fA-F]{64})$", text.splitlines()[0].strip()
        )
        assert m, f"{ck.name}: checksum line matches neither GNU nor BSD sha256sum form (G-2)"
        digest = m.group(1) if len(m.groups()) == 1 else m.group(2)
        got = hashlib.sha256(art.read_bytes()).hexdigest()
        assert got.lower() == digest.lower(), (
            f"{art.name}: sha256 {got} != recorded {digest}"
        )


@XFAIL_0403
def test_install_script_exists_and_posix_parseable():
    """04.03 text: 'install script'. URL and PATH are unpinned (G-1): we look
    for a committed install shell script (dist/install.sh or
    scripts/install.sh; override with APX_INSTALL_SH). The artifact must be
    POSIX-sh parseable — `sh -n` is the sh-level equivalent of `bash -n` —
    and must reference the antipythony binary name so it actually installs it.
    Today: no such file exists anywhere in the repo."""
    override = os.environ.get("APX_INSTALL_SH")
    cands = [Path(override)] if override else [
        REPO_ROOT / "dist" / "install.sh",
        REPO_ROOT / "scripts" / "install.sh",
        _dist_dir() / "install.sh",
    ]
    script = next((c for c in cands if c.exists()), None)
    assert script is not None, f"install script not found at any of {cands} (G-1)"
    proc = subprocess.run(
        ["/bin/sh", "-n", str(script)], capture_output=True, timeout=30
    )
    assert proc.returncode == 0, (
        f"install script fails POSIX-sh parse: {proc.stderr.decode(errors='replace')}"
    )
    body = script.read_text().lower()
    assert "antipythony" in body, "install script never names the antipythony binary"
    # checksum discipline in the installer is implied by 04.03 '+ checksums'
    # but its download URL/host is unpinned — assert only existence of SOME
    # verification hook (sha256 / checksum mention), not a specific URL (G-2).
    assert "sha256" in body or "checksum" in body, (
        "install script contains no sha256/checksum verification (04.03 pins "
        "checksums; URL unpinned — G-2)"
    )


@XFAIL_0403
def test_version_notices_embedding_matches_committed_file():
    """04.03 text verbatim: 'THIRD_PARTY_NOTICES embedded in binaries and
    emitted by `--version --notices`'. Run the release CLI (release/
    distribution build via APX_DIST_BIN, else APX_CLI resolution) and require
    the emitted bytes to be EXACTLY the committed THIRD_PARTY_NOTICES bytes
    (after trailing-newline normalization — the exact trailing-newline policy
    is unpinned, G-2). OBSERVED AT AUTHORING: scaffold `antipythony
    --version --notices` prints nothing (silent stub), so this fails."""
    override = os.environ.get("APX_DIST_BIN")
    cmd = shlex.split(override) if override else _cli_cmd()
    proc = subprocess.run(
        cmd + ["--version", "--notices"], capture_output=True, timeout=60,
        cwd=tempfile.mkdtemp(prefix="t15_notices_"),
    )
    assert proc.returncode == 0, f"--version --notices rc={proc.returncode}"
    emitted = proc.stdout
    assert emitted.strip(), "--version --notices emitted no bytes (stub CLI)"
    committed = NOTICES.read_bytes()
    assert emitted.rstrip(b"\n") == committed.rstrip(b"\n"), (
        "embedded notices != committed THIRD_PARTY_NOTICES bytes "
        f"(emitted {len(emitted)} vs committed {len(committed)})"
    )


@XFAIL_0403
def test_fresh_machine_scripted_gate():
    """04.03 Gate verbatim: 'fresh-machine install -> new -> add rich -> run ->
    build, scripted, green.' One end-to-end test; step table lives in
    plans/antipythony/tests/T-15-release-install.md. Fidelity notes:
    - 'fresh machine' is APPROXIMATED: stripped env (HOME=tmpdir, PATH kept
      only when uv is required), fresh temp cwd, fresh cache. Full VM-level
      freshness is a CI concern; runner image is unpinned (G-3).
    - 'add rich' is NETWORK-dependent (PyPI via uv; Plan 04 CLI surface `add
      <pkg>`, deps via uv subprocess). No local pure-wheel fixture exists at
      authoring (tests/fixtures/ holds parse .py only), so rich comes from the
      real resolver with override env APX_ADD_SPEC (set it to a local wheel
      spec to run offline); missing uv/network fails inside this xfail.
    - final asserts: `run` prints the scaffolded program's output; `build`
      produces an executable artifact whose standalone run matches (02.06/
      04.02 surfaces, cited via the Plan 04 verb table)."""
    env = {k: os.environ[k] for k in ("PATH", "LANG", "TMPDIR") if k in os.environ}
    work = Path(tempfile.mkdtemp(prefix="t15_fresh_"))
    env["HOME"] = str(work / "home")
    (work / "home").mkdir()
    cli = _cli_cmd()

    def step(args, cwd=work, timeout=300):
        proc = subprocess.run(
            cli + args, capture_output=True, cwd=cwd, env=env, timeout=timeout
        )
        assert proc.returncode == 0, (
            f"step {args} rc={proc.returncode}\n"
            f"stdout:\n{proc.stdout.decode(errors='replace')}\n"
            f"stderr:\n{proc.stderr.decode(errors='replace')}"
        )
        return proc

    # step 1 — install: the release CLI must be resolvable to a freshly
    # installed binary in the stripped HOME env (install script path unpinned,
    # G-1; we settle for: the resolved CLI runs at all under fresh-HOME).
    step(["--version"], timeout=30)
    # step 2 — new: scaffold myapp (Plan 04 surface: `antipythony new myapp`).
    step(["new", "myapp"])
    proj = work / "myapp"
    assert proj.exists(), "`new myapp` exited 0 but created no myapp/ dir"
    # step 3 — add rich: `antipythony add rich` per the verb table
    # (`antipythony add requests`; 04.03 Gate names `rich`).
    spec = os.environ.get("APX_ADD_SPEC", "rich").split()
    step(["add"] + spec, cwd=proj, timeout=600)
    lock = list(proj.glob("antipythony.lock*")) + list(proj.glob("*lock*"))
    assert lock, "`add` recorded no lock file in the project"
    # step 4 — run: scaffolded entry (verb table `antipythony run app.py`;
    # the scaffolded main file name is 04.01 surface — glob for the .py).
    entry = next(iter(sorted(proj.glob("*.py"))), None)
    assert entry is not None, "scaffold produced no Python entry file"
    runp = step(["run", entry.name], cwd=proj)
    assert runp.stdout, "`run` produced no stdout for the scaffolded program"
    # step 5 — build: verb table `antipythony build app.py` -> ./app.
    step(["build", entry.name], cwd=proj)
    app = proj / ("app" if sys.platform != "win32" else "app.exe")
    assert app.exists(), "`build` exited 0 but produced no ./app artifact"
    assert os.access(app, os.X_OK), "./app not executable"
    standalone = subprocess.run(
        [str(app)], capture_output=True, cwd=proj, env=env, timeout=120
    )
    assert standalone.returncode == 0 and standalone.stdout, (
        "built app does not run standalone on the fresh-HOME machine"
    )
