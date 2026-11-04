# T-14 — cli/cache (gates leaves 04.01 uv integration + 04.02 ap-cli verbs/build cache).
#
# Authorship: written from the spec pack + plan text ONLY —
# plans/antipythony/04-packaging-ecosystem/orchestrator.md (CLI surface block,
# Dependency model, Build cache key schema, 04.01 Expose/Gate, 04.02 Expose/
# Gate), docs/compat-contract.md (C-06..C-10 fail-loud + APX tokens; C-26),
# docs/spec/05-builtins-stdlib-v1.md (purity rules S-05-1..3), plans/
# antipythony/master.md (uv from v0; 3.14 pin), plans/antipythony/tests/
# README.md. No implementation source under crates/ was read.
# Per-test citations + CONTRACT-GAPS: plans/antipythony/tests/T-14-cli-cache.md.
#
# Structure:
#   GREEN NOW     — expectation-file validation (fixture toml/lock are valid
#                   TOML with the schema keys the tests demand), fixture
#                   purity, fake native-wheel buildability, C-08/C-10
#                   diagnostic-shape expectations, cache-layout constants,
#                   uv availability report.
#   XFAIL (04.01) — new/add verbs, toml/lock emission, pure-wheel filter at
#                   add time, missing-uv helpful error.
#   XFAIL (04.02) — run/run --no-cache, cache-key dimensions, cross-process
#                   cache survival, cache GC, reproducible build, wasm verb
#                   plumbing. The <200ms warm-run figure is MEASURED-AND-
#                   REPORTED (G-3, T-08 G-2 precedent); what is asserted is
#                   cache-hit DETERMINISM + --no-cache bypass.
#
# SELF-CONTAINMENT: tests never touch the network. The primary `add` fixture
# is a local pure-python package (file: path). The one PyPI-anchored test
# skips-with-reason when offline (recorded as CONTRACT-GAP G-7). A local path
# install that fails because uv needs to fetch build backend packages is
# likewise skipped-with-reason in-test.
#
# uv availability at authoring time: uv 0.11.29 present at
# /Users/caimlas/.local/bin/uv (measured, green test below). CLI-absence
# policy: tests that invoke the `antipythony` binary xfail on its
# FileNotFoundError (like T-09's _cli_cmd); uv itself is a real external
# tool that IS present, so uv-subprocess tests are NOT pre-skipped.

import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path

import pytest

try:
    import tomllib  # py3.11+; reference interpreter is 3.14
except ModuleNotFoundError:  # pragma: no cover
    tomllib = None

REPO_ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures"
TINY_PURE = FIXTURES / "tiny_pure"

# Cache layout given verbatim by Plan 04 §Design "Build cache" line.
CACHE_ROOT = Path.home() / ".cache" / "antipythony" / "build"

XFAIL_0401 = pytest.mark.xfail(
    strict=False, reason="awaits 04.01 uv integration (new/add verbs, toml+lock, add-time filter)"
)
XFAIL_0402 = pytest.mark.xfail(
    strict=False, reason="awaits 04.02 ap-cli verbs + content-addressed build cache"
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _cli_cmd() -> list:
    """The `antipythony` driver (Plan 04 §Design CLI surface). Resolution
    order mirrors T-09: APX_CLI override, PATH, target/{debug,release}.
    FileNotFoundError before the xfail marker turns the whole CLI half into
    expected-failure until the binary exists (tests/README.md step 2)."""
    override = os.environ.get("APX_CLI")
    if override:
        return shlex_split(override)
    found = shutil.which("antipythony")
    if found:
        return [found]
    for profile in ("debug", "release"):
        cand = REPO_ROOT / "target" / profile / "antipythony"
        if cand.exists():
            return [str(cand)]
    raise FileNotFoundError(
        "antipythony CLI not found (set APX_CLI=<cmdline> to override)"
    )


def shlex_split(s: str) -> list:
    import shlex

    return shlex.split(s)


def _clean_env(home: Path, path_value: str | None = None) -> dict:
    """Minimal env: HOME points at an isolated temp home so the real
    ~/.cache/antipythony is never touched and cache state is per-test
    (04.01 Gate 'on a clean cache'). PATH keeps uv reachable (uv is the
    plan-04 mechanism; stripping it is its own test)."""
    env = {
        "HOME": str(home),
        "LANG": os.environ.get("LANG", "C"),
        "TMPDIR": os.environ.get("TMPDIR", str(home)),
    }
    env["PATH"] = path_value if path_value is not None else os.environ.get("PATH", "")
    return env


def _run_cli(args, home: Path, cwd: Path, path_value=None, timeout=300):
    try:
        cmd = _cli_cmd()
    except FileNotFoundError as exc:
        # An absent binary is NOT a pass: raise inside the xfail so the test
        # marks XFAIL (awaiting impl), never XPASS on a FileNotFoundError.
        pytest.fail(str(exc))
    return subprocess.run(
        cmd + list(args),
        capture_output=True,
        cwd=cwd,
        env=_clean_env(home, path_value),
        timeout=timeout,
    )


def _parse_toml(path: Path) -> dict:
    assert tomllib is not None, "tomllib required (reference interpreter is 3.14)"
    with open(path, "rb") as f:
        return tomllib.load(f)


def _make_native_wheel(dest_dir: Path) -> Path:
    """Build a fixture wheel that CONTAINS a native component (.so) — the
    exact artifact the pure-wheel-only filter must refuse (04.01: 'Only
    pure-Python wheels are accepted in v1; a wheel with native components
    produces the C-extension diagnostic ... at ADD time'). Built in-test into
    a tmp dir so no binary blob is committed (corpus-only-grows safe)."""
    dest_dir.mkdir(parents=True, exist_ok=True)
    whl = dest_dir / "native_demo-0.1.0-py3-none-any.whl"
    so_bytes = b"\x7fELF fake native component - T-14 fixture, not loadable"
    with zipfile.ZipFile(whl, "w") as z:
        z.writestr("native_demo/__init__.py",
                   b"from native_demo._native import noop\n")
        z.writestr("native_demo/_native.cpython-314-x86_64-linux-gnu.so", so_bytes)
        z.writestr(
            "native_demo-0.1.0.dist-info/METADATA",
            "Metadata-Version: 2.1\nName: native-demo\nVersion: 0.1.0\n"
            "Summary: T-14 fixture wheel with a native component\n",
        )
        z.writestr(
            "native_demo-0.1.0.dist-info/WHEEL",
            "Wheel-Version: 1.0\nGenerator: t14-fixture\n"
            "Root-Is-Purelib: false\nTag: py3-none-any\n",
        )
        z.writestr("native_demo-0.1.0.dist-info/RECORD", "")
    return whl


def _make_pure_wheel(dest_dir: Path) -> Path:
    """Contrast fixture: a wheel with NO native member (pure-Python)."""
    dest_dir.mkdir(parents=True, exist_ok=True)
    whl = dest_dir / "pure_demo-0.1.0-py3-none-any.whl"
    with zipfile.ZipFile(whl, "w") as z:
        z.writestr("pure_demo/__init__.py", b"VALUE = 1\n")
        z.writestr(
            "pure_demo-0.1.0.dist-info/METADATA",
            "Metadata-Version: 2.1\nName: pure-demo\nVersion: 0.1.0\n",
        )
        z.writestr("pure_demo-0.1.0.dist-info/WHEEL",
                   "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n")
        z.writestr("pure_demo-0.1.0.dist-info/RECORD", "")
    return whl


_NATIVE_MEMBER = re.compile(r"(?i)\.(so|dylib|pyd|dll|o)$")
_CEXT_DIAG_RE = re.compile(  # C-08 Verify, verbatim regex shape
    r"(?s).*_native.*C extension.*(compat-contract|10-c-extensions).*"
)
_LOCATION_RE = re.compile(r"\S+:\d+:\d+")  # file:line:col (C-08/C-10)


def _cache_entries(home: Path) -> set:
    root = home / ".cache" / "antipythony" / "build"
    if not root.exists():
        return set()
    return {str(p.relative_to(root)) for p in root.rglob("*") if p.is_file()}


# ---------------------------------------------------------------------------
# HALF 1 — GREEN NOW: expectation files + fixture self-validation.
# No implementation needed; these are the oracles the xfail half compares to.
# ---------------------------------------------------------------------------


def test_expected_toml_fixture_parses():
    """04.01 Expose 'toml+lock schema': our expectation file for
    antipythony.toml must itself be valid TOML carrying the key paths the
    schema test demands ([project] name/version/requires_python, [build]
    backend/opt_level, [[dependency]] name). Cites: Plan 04 §Design, G-1."""
    data = _parse_toml(FIXTURES / "expected_antipythony.toml")
    assert data["project"]["name"] == "myapp"
    assert data["project"]["version"] == "0.1.0"
    assert data["project"]["requires_python"] == ">=3.14"  # C-01 / master.md 3.14 pin
    assert data["build"]["backend"] == "native"  # cache-key dim 'backend' (04.02)
    assert isinstance(data["build"]["opt_level"], int)  # cache-key dim 'opt level'
    deps = data["dependency"]
    assert len(deps) == 1 and deps[0]["name"] == "rich"  # 04.01 Gate example dep


def test_expected_lock_fixture_parses():
    """04.01 'toml+lock schema': the lock expectation is valid TOML with a
    uv-provenance marker and a pinned-version [[package]] entry. Cites:
    Plan 04 §Design 'resolve+lock via uv', G-1."""
    data = _parse_toml(FIXTURES / "expected_antipythony.lock")
    assert data["metadata"]["created_by"] == "uv"
    assert isinstance(data["metadata"]["lock_version"], int)
    pkgs = data["package"]
    assert len(pkgs) == 1 and pkgs[0]["name"] == "rich"
    assert pkgs[0]["version"], "lock must pin a version (04.01 resolve+lock)"


def test_fixture_tiny_pure_is_self_contained_package():
    """The add-able local dep: valid pyproject, importable-name package,
    pure (no C-extension import — spec 05 S-05-1..3), zero external runtime
    deps. Cites: 04.01 Gate 'one pure-Python dep', S-05-2."""
    py = _parse_toml(TINY_PURE / "pyproject.toml")
    assert py["project"]["name"] == "tiny-pure"
    assert py["project"]["requires-python"] == ">=3.14"
    src = (TINY_PURE / "tiny_pure" / "__init__.py").read_text()
    assert not re.search(r"import\s+(numpy|pydantic|_socket|_ssl|_json)\b", src)
    assert (TINY_PURE / "tiny_pure" / "__init__.py").exists()


def test_app_seed_only_imports_fixture_dep():
    """Scaffold app source imports tiny_pure so an add/run e2e proves the
    lock closure reaches compilation (04.01 Gate). Cites: Plan 04 §Design
    'scaffold: myapp.py + antipythony.toml'."""
    imports = re.findall(r"^(?:import|from)\s+([\w.]+)",
                         (FIXTURES / "app_seed.py").read_text(), re.M)
    assert imports == ["tiny_pure"]


def test_native_wheel_fixture_builds_and_is_detectably_impure():
    """The refusal fixture must be mechanically detectable as non-pure:
    member list contains a .so. This predicate is the oracle for the add-time
    filter test. Cites: 04.01 'pure-wheel-only acceptance filter', C-06."""
    with tempfile.TemporaryDirectory() as td:
        whl = _make_native_wheel(Path(td))
        names = zipfile.ZipFile(whl).namelist()
        assert any(_NATIVE_MEMBER.search(n) for n in names)
        pure = _make_pure_wheel(Path(td))
        assert not any(_NATIVE_MEMBER.search(n) for n in zipfile.ZipFile(pure).namelist())


def test_cext_diagnostic_expectation_matches_c08_regex():
    """C-08/C-10 diagnostic-shape oracle (fixture-side, not CLI-side): a
    conforming add-time refusal line — APX-CEXT token + full module name +
    'C extension' + policy pointer + file:line:col — passes the C-08 Verify
    regex. If this test fails, our expectations, not the CLI, are wrong."""
    sample = (
        "APX-CEXT: native_demo._native is a C extension module; v1 refuses it. "
        "See docs/compat-contract.md C-07 and plans/antipythony/10-c-extensions-"
        "stretch/orchestrator.md (app.py:1:1)"
    )
    assert "APX-CEXT" in sample
    assert _CEXT_DIAG_RE.search(sample.replace("_native", "_native"))
    assert _LOCATION_RE.search(sample)


def test_cache_root_layout_constant():
    """Plan 04 §Design verbatim: 'Build cache: ~/.cache/antipythony/build/
    <hash>'. Asserts the layout we will look entries under; the <hash> INPUT
    RECIPE is unpinned (G-2) so this stops at the directory shape."""
    assert str(CACHE_ROOT).endswith("/.cache/antipythony/build")
    assert CACHE_ROOT.is_absolute()


def test_uv_availability_report():
    """04.01 Expose 'detect uv, pin minimum version': record the real uv
    present for the uv-subprocess half. REPORT-only (version pinned by the
    leaf is unknown, G-5); absence is a gap, not a failure (skips downstream)."""
    uv = shutil.which("uv")
    if uv is None:
        pytest.skip("uv not on PATH — 04.01 subprocess tests would skip anyway; recorded as env note")
    out = subprocess.run([uv, "--version"], capture_output=True, timeout=10)
    assert out.returncode == 0
    print(f"T-14 UV REPORT: {out.stdout.decode().strip()} at {uv}")


# ---------------------------------------------------------------------------
# HALF 2 — 04.01 GATE (xfail, strict=False): uv-backed verbs + add-time filter.
# ---------------------------------------------------------------------------


@XFAIL_0401
def test_new_scaffolds_project_files(tmp_path):
    """Plan 04 surface: `antipythony new myapp` -> scaffold: myapp.py +
    antipythony.toml. Both files must exist; the toml must carry the schema
    keys (G-1: names are normative expectations from the fixture oracle)."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "ws"
    proj.mkdir()
    proc = _run_cli(["new", "myapp"], home, proj)
    assert proc.returncode == 0, proc.stderr.decode(errors="replace")
    app_toml = proj / "myapp" / "antipythony.toml"
    assert app_toml.exists(), "new must write antipythony.toml"
    assert (proj / "myapp" / "myapp.py").exists(), "new must scaffold myapp.py"
    data = _parse_toml(app_toml)
    for path in (["project", "name"], ["project", "version"],
                 ["project", "requires_python"]):
        node = data
        for key in path:
            assert key in node, f"toml missing schema key project.{key} (04.01, G-1)"
            node = node[key]


@XFAIL_0401
def test_add_local_pure_dep_writes_lock(tmp_path):
    """04.01 Gate: add flow with one pure-Python dep, run END-TO-END OFFLINE
    via local file: path (tiny-pure fixture; registry fetch would need network
    — G-7). Adds must record the dep in antipythony.toml and pin it in
    antipythony.lock; lock must be valid TOML with a non-empty version pin."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "myapp"
    proj.mkdir()
    _run_cli(["new", "myapp"], home, tmp_path)  # best-effort; add below works in proj either way
    shutil.copy(FIXTURES / "app_seed.py", proj / "myapp.py")
    proc = _run_cli(["add", f"file:{TINY_PURE}"], home, proj)
    if proc.returncode != 0 and re.search(
        r"(?i)network|offline|could not (fetch|resolve|download)|no such|connection",
        proc.stderr.decode(errors="replace"),
    ):
        # A local path install that still reaches the index for build backend
        # packages (setuptools) is network-bound: skip-with-reason, G-7.
        pytest.skip("uv local-file add required network fetch — self-contained run impossible (G-7)")
    assert proc.returncode == 0, proc.stderr.decode(errors="replace")
    lock = proj / "antipythony.lock"
    assert lock.exists(), "add must write antipythony.lock (Plan 04 §Design)"
    data = _parse_toml(lock)
    blob = repr(data).lower()
    assert "tiny" in blob, "lock must record tiny-pure"
    toml_data = _parse_toml(proj / "antipythony.toml")
    assert "tiny" in repr(toml_data).lower(), "add must record dep in antipythony.toml"


@XFAIL_0401
def test_add_native_wheel_rejected_at_add_time_with_cext_diag(tmp_path):
    """04.01: 'a wheel with native components produces the C-extension
    diagnostic ... at ADD time, not build time (fail early)'. Assert the
    refusal trio on the ADD verb: nonzero exit, APX-CEXT token (C-10), the
    C-08 regex (module name + 'C extension' + policy pointer) + file:line
    shape; and NO lock entry written (C-09 no silent stub, C-11 total refusal)."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "myapp"
    proj.mkdir()
    _run_cli(["new", "myapp"], home, tmp_path)
    whl = _make_native_wheel(tmp_path)
    proc = _run_cli(["add", str(whl)], home, proj)
    err = proc.stderr.decode(errors="replace")
    assert proc.returncode != 0, "native-component wheel must be refused at add"
    assert "APX-CEXT" in err, f"C-10 token missing: {err}"
    assert re.search(r"(?i)native_demo.*C extension|c-extension", err), err
    assert re.search(r"compat-contract|10-c-extensions", err), f"C-08 policy pointer missing: {err}"
    if (proj / "antipythony.lock").exists():
        assert "native" not in repr(_parse_toml(proj / "antipythony.lock")).lower(), (
            "refused dep must not enter the lock (C-09/C-11)"
        )


@XFAIL_0401
def test_add_uv_registry_flow_pypi_offline_skip(tmp_path):
    """04.01 Gate verbatim: 'new/add/run flow with one pure-Python dep
    (e.g. rich)'. This is the PyPI-anchored variant: SKIP-with-reason when
    offline (network policy — G-7); the offline local-file variant above is
    the self-contained equivalent."""
    import socket

    try:
        socket.getaddrinfo("pypi.org", 443)
    except OSError:
        pytest.skip("no DNS for pypi.org — network-dependent test (G-7); "
                    "offline coverage via file: fixture add")
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "myapp"
    proj.mkdir()
    _run_cli(["new", "myapp"], home, tmp_path)
    proc = _run_cli(["add", "rich"], home, proj)
    assert proc.returncode == 0, proc.stderr.decode(errors="replace")
    assert (proj / "antipythony.lock").exists()


@XFAIL_0401
def test_add_without_uv_on_path_errors_helpfully(tmp_path):
    """04.01 Expose: 'helpful error if absent'. PATH stripped to a dir with
    no uv (asserted precondition); `add` must exit nonzero and its stderr
    must name 'uv' and tell the user to install it (G-4: exact wording
    unspecced; the 'uv' mention is the minimal helpfulness claim)."""
    iso = tmp_path / "emptybin"
    iso.mkdir()
    assert shutil.which("uv", path=str(iso)) is None, "isolation dir unexpectedly has uv"
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "myapp"
    proj.mkdir()
    proc = _run_cli(["add", f"file:{TINY_PURE}"], home, proj, path_value=str(iso))
    err = (proc.stderr + proc.stdout).decode(errors="replace")
    assert proc.returncode != 0, f"add without uv must fail (04.01): {err}"
    assert "uv" in err.lower(), f"error must name uv (G-4): {err}"


# ---------------------------------------------------------------------------
# HALF 3 — 04.02 GATE (xfail, strict=False): verbs + content-addressed cache.
# ---------------------------------------------------------------------------


def _warm_project(tmp_path):
    """Create home + a project with app source; returns (home, proj).
    The run-half needs tiny_pure importable: rely on the add flow if present,
    else the app runs without deps (cache behaviour is dep-independent)."""
    home = tmp_path / "home"
    home.mkdir()
    proj = tmp_path / "myapp"
    proj.mkdir()
    shutil.copy(FIXTURES / "app_seed.py", proj / "myapp.py")
    return home, proj


@XFAIL_0402
def test_run_cold_then_warm_same_cache_entry(tmp_path):
    """04.02: 'run uses cache' + content-addressed key. Assert cache-hit
    DETERMINISM, not timing (G-3): the cold run creates a cache entry; a warm
    run of the same input must NOT add a second entry (same input hash ->
    same <hash> dir). Program output byte-equal cold vs warm (C-26 output
    bytes: the cached artifact must run identically) — and both runs must
    ACTUALLY execute the program (bytes check guards against a silent
    exit-0 stub passing a cache-invariance-only assertion)."""
    home, proj = _warm_project(tmp_path)
    cold = _run_cli(["run", "myapp.py"], home, proj)
    assert cold.returncode == 0, cold.stderr.decode(errors="replace")
    assert b"hello, antipythony" in cold.stdout, "run must execute the program"
    entries_after_cold = _cache_entries(home)
    assert entries_after_cold, "run must populate ~/.cache/antipythony/build (G-2 layout)"
    warm = _run_cli(["run", "myapp.py"], home, proj)
    assert warm.returncode == 0
    assert b"hello, antipythony" in warm.stdout
    assert warm.stdout == cold.stdout, "warm run output bytes differ from cold (C-26)"
    entries_after_warm = _cache_entries(home)
    assert entries_after_warm == entries_after_cold, (
        f"warm run created new cache entries: {entries_after_warm - entries_after_cold}"
    )


@XFAIL_0402
def test_run_no_cache_bypasses_cache(tmp_path):
    """Plan 04 surface + 04.02: `--no-cache`. With --no-cache the run must
    succeed, produce the program's bytes, and must not consume/populate the
    keyed cache. Cold run first to establish a populated baseline (an empty
    cache makes the no-change assertion vacuous — stub-binary trap), then
    --no-cache must leave the entry set identical while still running."""
    home, proj = _warm_project(tmp_path)
    cold = _run_cli(["run", "myapp.py"], home, proj)
    assert cold.returncode == 0, cold.stderr.decode(errors="replace")
    assert b"hello, antipythony" in cold.stdout, "run must execute the program"
    before = _cache_entries(home)
    assert before, "cold run must populate the cache (baseline for bypass check)"
    proc = _run_cli(["run", "--no-cache", "myapp.py"], home, proj)
    assert proc.returncode == 0, proc.stderr.decode(errors="replace")
    assert b"hello, antipythony" in proc.stdout, "--no-cache must still run the program"
    after = _cache_entries(home)
    assert after == before, f"--no-cache touched the cache: {after ^ before}"


@XFAIL_0402
def test_cache_survives_across_processes(tmp_path):
    """Task design: cache is on-disk content-addressed state, not in-memory:
    two SEPARATE CLI invocations on the same input share entries; the second
    sees the first's entry (entry set stable across independent processes)."""
    home, proj = _warm_project(tmp_path)
    p1 = _run_cli(["run", "myapp.py"], home, proj)
    assert p1.returncode == 0
    e1 = _cache_entries(home)
    assert e1
    _run_cli(["run", "myapp.py"], home, proj)  # second, independent process
    e2 = _cache_entries(home)
    assert e1 <= e2, "entries vanished between processes"
    new = e2 - e1
    assert not new, f"second process duplicated cache entries: {new}"


@XFAIL_0402
def test_warm_run_timing_is_measured_and_reported(tmp_path):
    """04.02 Gate: 'second run of same input <200ms to execution'. MEASURED,
    NOT ASSERTED (G-3, T-08 G-2 precedent): process-spawn+compile-walk
    cache is not contractual here; what is asserted is the DETERMINISTIC
    part (no new entries + real execution). The ms figure is printed."""
    home, proj = _warm_project(tmp_path)
    cold = _run_cli(["run", "myapp.py"], home, proj)
    assert cold.returncode == 0
    assert b"hello, antipythony" in cold.stdout, "cold run must execute the program"
    t0 = time.monotonic()
    warm = _run_cli(["run", "myapp.py"], home, proj)
    ms = (time.monotonic() - t0) * 1000
    assert warm.returncode == 0
    assert b"hello, antipythony" in warm.stdout, "warm run must execute the program"
    print(f"T-14 WARM-RUN REPORT: second `run` to completion = {ms:.0f} ms "
          f"(gate text says <200ms to execution; report-only, G-3)")


@XFAIL_0402
def test_cache_key_changes_with_input_hash(tmp_path):
    """Cache key schema dim (input hash): edit the source, the next run must
    add a NEW keyed entry (different input hash -> different <hash> dir), not
    reuse the old one; and run output must reflect the edit (stale-cache bug
    detector)."""
    home, proj = _warm_project(tmp_path)
    _run_cli(["run", "myapp.py"], home, proj)
    before = _cache_entries(home)
    assert before
    (proj / "myapp.py").write_text((proj / "myapp.py").read_text() + "\nprint('edited')\n")
    proc = _run_cli(["run", "myapp.py"], home, proj)
    assert proc.returncode == 0
    assert b"edited" in proc.stdout, "input change did not rebuild (input-hash dim)"
    after = _cache_entries(home)
    assert after != before, "no new cache entry for changed input hash"


@XFAIL_0402
def test_cache_key_changes_with_opt_level(tmp_path):
    """Cache key schema dims (opt level) [and backend, G-2: no opt-level flag
    is specced on the CLI surface — probe via toml [build] opt_level if the
    flag form does not exist]. Editing the opt level must yield a distinct
    cache entry."""
    home, proj = _warm_project(tmp_path)
    _run_cli(["run", "myapp.py"], home, proj)
    before = _cache_entries(home)
    assert before
    toml_path = proj / "antipythony.toml"
    if toml_path.exists():
        # G-2 fallback: mutate the toml opt_level field (schema name unpinned).
        text = toml_path.read_text()
        if "opt_level" in text:
            toml_path.write_text(re.sub(r"opt_level\s*=\s*\d+",
                                         lambda m: "opt_level = 0", text))
    proc = _run_cli(["run", "--opt-level", "0", "myapp.py"], home, proj)
    if proc.returncode != 0 and "--opt-level" in (proc.stderr.decode() + proc.stdout.decode()):
        proc = _run_cli(["run", "myapp.py"], home, proj)  # toml-driven variant
    assert proc.returncode == 0, proc.stderr.decode(errors="replace")
    after = _cache_entries(home)
    assert after != before, "opt-level change produced identical cache key (04.02 key schema)"


@XFAIL_0402
def test_cache_gc_verb(tmp_path):
    """04.02 Expose: 'cache GC subcommand'. `antipythony cache gc` must exist
    and exit 0 after a warm project, on the documented cache root, and must
    not break subsequent real execution. (Entry eviction policy itself is
    unspecced — G-8; only verb existence + clean exit + a subsequent run that
    still produces the program's bytes are checked.)"""
    home, proj = _warm_project(tmp_path)
    cold = _run_cli(["run", "myapp.py"], home, proj)
    assert cold.returncode == 0
    assert b"hello, antipythony" in cold.stdout
    proc = _run_cli(["cache", "gc"], home, proj)
    assert proc.returncode == 0, proc.stderr.decode(errors="replace")
    r2 = _run_cli(["run", "myapp.py"], home, proj)
    assert r2.returncode == 0, "gc must not break subsequent runs"
    assert b"hello, antipythony" in r2.stdout, "post-gc run must execute the program"


@XFAIL_0402
def test_build_reproducible_byte_identical_across_clean_caches(tmp_path):
    """04.02 Gate verbatim: 'build output reproducible byte-for-byte across
    two clean caches (documented exceptions: linker build-id — zeroed)'.
    Two builds in two isolated HOMEs (clean caches), same input -> identical
    artifact bytes. (Cross-refs T-09 test_reproducible_build_byte_identity,
    which marks the same property under 04.02.)"""
    blobs = []
    for tag in ("a", "b"):
        home = tmp_path / tag / "home"
        home.mkdir(parents=True)
        proj = tmp_path / tag / "myapp"
        proj.mkdir()
        shutil.copy(FIXTURES / "app_seed.py", proj / "myapp.py")
        out = proj / "app"
        proc = _run_cli(["build", "myapp.py", "-o", str(out)], home, proj)
        assert proc.returncode == 0, proc.stderr.decode(errors="replace")
        assert out.exists()
        blobs.append(out.read_bytes())
    assert blobs[0] == blobs[1], "build not byte-reproducible across clean caches (04.02)"


@XFAIL_0402
def test_run_wasm_verb_plumbing(tmp_path):
    """Plan 04 surface: `antipythony run --wasm app.py` (compile to wasm +
    execute via ap-host). Plumbing verb exists per surface; full parity is
    Plan 03/04.04 territory — here only the native-surface contract: exit 0
    and the program's bytes. Marked 04.02: the verb+cache half is gated here
    per the task split (04.04 ships the runner)."""
    home, proj = _warm_project(tmp_path)
    proc = _run_cli(["run", "--wasm", "myapp.py"], home, proj)
    assert proc.returncode == 0, proc.stderr.decode(errors="replace")
    assert b"hello, antipythony" in proc.stdout


# Network/build-backend caveat for the whole CLI half is documented in
# plans/antipythony/tests/T-14-cli-cache.md CONTRACT-GAPS G-7.
