# T-09 — binary conformance suite (gates leaf 02.06 AoT static binary).
#
# Authorship: written from the contract + spec pack + leaf gate text ONLY
# (docs/compat-contract.md C-01..C-03/C-23..C-31, docs/spec/06 §6.4-6.5,
# plans/antipythony/master.md, 02-native-backend/orchestrator.md 02.06,
# 00-foundation-contract/orchestrator.md 00.04, 04-packaging-ecosystem/
# orchestrator.md verbs + 04.02). No implementation source under crates/ was
# read. Per-test citations and CONTRACT-GAPS: plans/antipythony/tests/
# T-09-binary-conformance.md.
#
# Structure:
#   GREEN NOW   — inline-case expected triples re-verified against the
#                 reference CPython; slice manifest; ap-conform harness
#                 self-comparison over the exact 30-case slice.
#   XFAIL (02.06) — build seam, standalone binary run, PATH isolation, 30-case
#                 ap-conform-against-binary slice, C-28/C-29 exit codes in
#                 binary form, size bound (mode-gated, G-2), argv/cwd probes.
#   XFAIL (04.02) — reproducible-build byte identity (separate marker, G-3).

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
HERE = Path(__file__).resolve().parent
CASES = HERE / "cases"
EXPECTED = CASES / "expected"
CORPUS = REPO_ROOT / "tests" / "corpus"

# Reference CPython (C-01: pinned 3.14.0; recorded/verified here on 3.14.7 —
# provisional per GAP-4, same open item as T-06 GAP-7).
# Reference discovery: unified env+PATH ladder (robustness-ci 01.01); see
# tests/tools/_ref_python.py for the AP_REQUIRE_REFERENCE=1 policy.
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
from _ref_python import reference_or_skip  # noqa: E402

XFAIL_0206 = pytest.mark.xfail(
    strict=False, reason="awaits 02.06 AoT static binary (build verb + artifact)"
)
XFAIL_0402 = pytest.mark.xfail(
    strict=False,
    reason="cross-ref GAP-3: byte-identity SHOULD hold only once 04.02 "
    "(linker build-id zeroing / reproducible-build gate) ships",
)

# ---------------------------------------------------------------------------
# Pinned 30-case slice (02.06 Gate: "conformance suite runs against the BINARY,
# not the JIT, for a 30-case slice"). Composition: 10 values, 10 classes,
# 5 exceptions, 5 gc. Paths are frozen here; the manifest test keeps them honest.
# ---------------------------------------------------------------------------
SLICE_30 = [
    "values/V001-bigint-exactness.py",
    "values/V003-floordiv-mod-signs.py",
    "values/V008-bool-is-int.py",
    "values/V013-slice-algorithm.py",
    "values/V014-fstring-formats.py",
    "values/V040-dict-insertion-order.py",
    "values/V047-truthiness-table.py",
    "values/V051-eq-vs-identity.py",
    "values/V061-float-repr-shortest.py",
    "values/V068-float-format-specs.py",
    "classes/c001_class_basics.py",
    "classes/c003_new_init_order.py",
    "classes/c008_data_descriptor_priority.py",
    "classes/c011_property_setter.py",
    "classes/c016_slots_messages.py",
    "classes/c019_mro_examples.py",
    "classes/c022_super_diamond_chase.py",
    "classes/c024_descriptor_kinds.py",
    "classes/c027_binop_dispatch.py",
    "classes/c033_isinstance_issubclass.py",
    "exceptions/e001_raise_requires_baseexception.py",
    "exceptions/e002_hierarchy_structure.py",
    "exceptions/e005_custom_hierarchy.py",
    "exceptions/e007_raise_bare_class.py",
    "exceptions/e015_exception_class_shape.py",
    "gc/gc001_del_runs_at_collect.py",
    "gc/gc002_finalizer_order_one_generation.py",
    "gc/gc003_refcycle_with_del.py",
    "gc/gc004_weakref_callback_on_collect.py",
    "gc/gc011_collect_returns_and_garbage.py",
]

# Inline exit-code cases exercised against the BINARY (argv/cwd probes x011/x013
# have their own tests; the reference triples for ALL 13 are re-verified green).
EXIT_CASES = [
    "x001_exit_int",
    "x002_exit_negative",
    "x003_exit_str",
    "x004_exit_bare",
    "x005_exit_none",
    "x006_fall_through",
    "x007_exit_object",
    "x008_exit_list",
    "x009_uncaught_valueerror",
    "x010_thread_systemexit",
    "x012_keyboard_interrupt",
]

ALL_INLINE = sorted(p.stem for p in CASES.glob("*.py"))

# Extra argv for reference re-runs of probe cases (canonical recording form:
# `python3.14 <name>.py [args]` with cwd = cases dir).
REF_ARGS = {"x011_argv_tail": ["alpha", "2"]}

# Traceback-bearing expected files: the File-line path is normalized to
# <PATH> in the recorded bytes (the binary's embedded program has no script
# path, so path text is never diffed — masking per spec 06 S-06-7 caveat).
PATH_NORMALIZED = {"x009_uncaught_valueerror", "x012_keyboard_interrupt"}


def _norm_stderr(data: bytes) -> bytes:
    return re.sub(rb'  File "[^"]*",', b'  File "<PATH>",', data)


def _posix_rc(returncode: int) -> int:
    """Signal death -> 128+signo, the shell `$?` a real user sees (C-29 row)."""
    return returncode if returncode >= 0 else 128 + abs(returncode)


def _run_ref_case(stem: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [reference_or_skip(), stem + ".py"] + REF_ARGS.get(stem, []),
        capture_output=True,
        cwd=CASES,
        timeout=60,
    )


def _clean_env(path_value=None) -> dict:
    env = {k: os.environ[k] for k in ("HOME", "LANG", "TMPDIR") if k in os.environ}
    env["PATH"] = path_value if path_value is not None else os.environ.get("PATH", "")
    return env


def _ap_conform_base() -> list:
    override = os.environ.get("AP_CONFORM")
    if override:
        return shlex.split(override)
    found = shutil.which("ap-conform")
    if found:
        return [found]
    for profile in ("debug", "release"):
        cand = REPO_ROOT / "target" / profile / "ap-conform"
        if cand.exists():
            return [str(cand)]
    raise FileNotFoundError(
        "ap-conform not built/found (set AP_CONFORM=<cmdline> to override)"
    )


def _cli_cmd() -> list:
    """The `antipythony` driver (build verb per Plan 04 surface + 02.06 Expose)."""
    override = os.environ.get("APX_CLI")
    if override:
        return shlex.split(override)
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


_BUILD_DIR = None


def _build_root() -> Path:
    global _BUILD_DIR
    if _BUILD_DIR is None:
        _BUILD_DIR = Path(tempfile.mkdtemp(prefix="t09_build_"))
    return _BUILD_DIR


def build_binary(script: Path, tag: str) -> Path:
    """`antipythony build <script> -o <tmp>/<tag>` (02.06 Expose verbatim), then
    require the artifact to EXIST and be executable — a zero-exit build that
    produces no artifact is a seam failure (GAP-6), not a pass."""
    out = _build_root() / tag
    proc = subprocess.run(
        _cli_cmd() + ["build", str(script), "-o", str(out)],
        capture_output=True,
        cwd=REPO_ROOT,
        env=_clean_env(),
        timeout=300,
    )
    assert proc.returncode == 0, (
        f"build failed rc={proc.returncode}\n"
        f"stdout:\n{proc.stdout.decode(errors='replace')}\n"
        f"stderr:\n{proc.stderr.decode(errors='replace')}"
    )
    assert out.exists(), f"build exited 0 but produced no artifact at {out} (GAP-6)"
    assert os.access(out, os.X_OK), f"artifact {out} is not executable"
    return out


def run_binary(app: Path, args=(), cwd=None, path_env=None) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(app)] + list(args),
        capture_output=True,
        cwd=cwd or REPO_ROOT,
        env=_clean_env(path_env),
        timeout=120,
    )


def compare_case(app: Path, stem: str) -> None:
    """Byte-compare the binary's 3 channels against the pinned reference triple
    (C-27; exit-code rows C-28/C-29/C-30)."""
    exp_out = (EXPECTED / (stem + ".stdout")).read_bytes()
    exp_err = _norm_stderr((EXPECTED / (stem + ".stderr")).read_bytes())
    exp_rc = int((EXPECTED / (stem + ".rc")).read_text().strip())
    proc = run_binary(app)
    got_err = _norm_stderr(proc.stderr) if stem in PATH_NORMALIZED else proc.stderr
    assert proc.stdout == exp_out, (
        f"{stem}: stdout drift\n got={proc.stdout!r}\n exp={exp_out!r}"
    )
    assert got_err == exp_err, f"{stem}: stderr drift\n got={got_err!r}\n exp={exp_err!r}"
    assert _posix_rc(proc.returncode) == exp_rc, (
        f"{stem}: exit code {_posix_rc(proc.returncode)} != pinned {exp_rc}"
    )


# ---------------------------------------------------------------------------
# HALF 1 — GREEN NOW: reference-anchored self-validation, no implementation.
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("stem", ALL_INLINE)
def test_expected_triples_reproduce(stem):
    """Each pinned triple in cases/expected/ is re-measured against the
    reference build on every run — drift (GAP-4) becomes a loud failure here,
    not a silent stale anchor. Cites: C-28, C-29, C-30, S-06-11, S-06-12."""
    proc = _run_ref_case(stem)
    exp_out = (EXPECTED / (stem + ".stdout")).read_bytes()
    exp_err = (EXPECTED / (stem + ".stderr")).read_bytes()
    exp_rc = (EXPECTED / (stem + ".rc")).read_text()
    assert proc.stdout == exp_out, f"{stem}: stdout drift {proc.stdout!r}"
    got_err = _norm_stderr(proc.stderr)
    assert got_err == exp_err, f"{stem}: stderr drift {got_err!r} != {exp_err!r}"
    assert f"{_posix_rc(proc.returncode)}\n" == exp_rc, (
        f"{stem}: rc {_posix_rc(proc.returncode)} != recorded {exp_rc.strip()}"
    )


@pytest.mark.parametrize("stem", ALL_INLINE)
def test_inline_case_cites_contract(stem):
    """tests/README.md corpus rule: every case header comment cites clause ids."""
    text = (CASES / (stem + ".py")).read_text()
    cited = re.findall(r"C-\d{2}|S-06-\d+|02\.06|04\.02", text)
    assert cited, f"{stem}: no clause citations found in file"


@pytest.mark.parametrize("rel", SLICE_30)
def test_slice_manifest_case_exists(rel):
    """The 30 pinned slice paths exist and are read-only references (corpus
    only grows — C-33; this leaf never modifies tests/corpus/)."""
    p = CORPUS / rel
    assert p.exists(), f"slice case missing: {p}"
    assert p.suffix == ".py"


def test_slice_composition_is_10_10_5_5():
    from collections import Counter

    c = Counter(rel.split("/")[0] for rel in SLICE_30)
    assert c == Counter({"values": 10, "classes": 10, "exceptions": 5, "gc": 5})
    assert len(set(SLICE_30)) == 30


def test_recorded_expected_streams_are_clean_outside_gc():
    """The 25 non-gc slice cases: rc 0 + empty stderr by construction (their
    expected files ship from T-05/T-06). gc expected files were pending at
    authoring (GAP-1), so only the 25 are checked file-wise; the gc binary
    gate rides on ap-conform live reference capture instead."""
    checked = 0
    for rel in SLICE_30:
        if rel.startswith("gc/"):
            continue
        stem = Path(rel).stem
        slice_dir = CORPUS / rel.split("/")[0]
        exp = slice_dir / "expected"
        rc_f = next(exp.glob(stem + ".rc"), None) or next(exp.glob(stem + ".exit"), None)
        err_f = next(exp.glob(stem + ".err"), None) or next(exp.glob(stem + ".stderr"), None)
        out_f = next(exp.glob(stem + ".out"), None) or next(exp.glob(stem + ".stdout"), None)
        assert rc_f and err_f and out_f, f"{rel}: expected triple incomplete"
        assert rc_f.read_text().strip() == "0", f"{rel}: rc != 0"
        assert err_f.read_bytes() == b"", f"{rel}: stderr not empty"
        checked += 1
    assert checked == 25


def test_harness_binary_resolvable():
    """00.04: the ap-conform runner exists and responds to its documented CLI."""
    base = _ap_conform_base()
    proc = subprocess.run(base + ["run", "--help"], capture_output=True, timeout=30)
    assert proc.returncode == 0
    assert b"--corpus" in proc.stdout and b"--impl" in proc.stdout


@pytest.mark.parametrize("rel", SLICE_30)
def test_ap_conform_self_comparison_green_on_slice(rel):
    """Harness contract proof (C-24 Verify, 00.04 Gate): with --impl python3.14
    the exact 30-case slice is green, so reference bytes for every slice case
    (including the gc five with no recorded files yet — GAP-1) are live-verified
    before any 02.06 binary is blamed or credited."""
    cmd = _ap_conform_base() + [
        "run",
        "--corpus",
        str(CORPUS / rel),
        "--impl",
        reference_or_skip(),
    ]
    proc = subprocess.run(cmd, capture_output=True, cwd=REPO_ROOT, timeout=120)
    assert proc.returncode == 0, (
        f"{rel}: self-comparison not green rc={proc.returncode}\n"
        f"{proc.stdout.decode(errors='replace')}\n{proc.stderr.decode(errors='replace')}"
    )


# ---------------------------------------------------------------------------
# HALF 2 — 02.06 GATE (xfail, strict=False). The BINARY is the artifact.
# ---------------------------------------------------------------------------


@XFAIL_0206
def test_build_hello_succeeds():
    """02.06 Expose: `antipythony build app.py -o app` emits an executable."""
    app = build_binary(CORPUS / "001_hello.py", "hello_app")
    assert app.exists()


@XFAIL_0206
def test_binary_runs_hello_program():
    """02.06 Gate part 1: `./app` runs the hello-world program byte-identically
    to the reference on all three channels (C-27, C-31 AoT invisibility)."""
    script = CORPUS / "001_hello.py"
    ref = subprocess.run([reference_or_skip(), str(script)], capture_output=True, cwd=REPO_ROOT, timeout=60)
    app = build_binary(script, "hello_app")
    proc = run_binary(app)
    assert proc.stdout == ref.stdout, f"stdout {proc.stdout!r} != {ref.stdout!r}"
    assert proc.stderr == ref.stderr, f"stderr {proc.stderr!r} != {ref.stderr!r}"
    assert proc.returncode == ref.returncode


@XFAIL_0206
def test_binary_runs_classes_corpus_program():
    """02.06 Gate part 2: a classes corpus program (c001) runs in the binary
    with bytes == the recorded T-06 expected anchor."""
    script = CORPUS / "classes" / "c001_class_basics.py"
    exp_out = (script.parent / "expected" / "c001_class_basics.out").read_bytes()
    app = build_binary(script, "c001_app")
    proc = run_binary(app)
    assert proc.stdout == exp_out, f"stdout drift: {proc.stdout!r}"
    assert proc.stderr == b""
    assert proc.returncode == 0


@XFAIL_0206
def test_binary_runs_under_path_isolation():
    """GAP-7 / Go-style UX: a static binary needs no Python interpreter. Run
    with PATH=/usr/bin:/bin — asserted to contain no python3.14 — and require
    the same 3-channel bytes as the reference (C-27, C-31)."""
    iso_path = "/usr/bin:/bin"
    assert shutil.which("python3.14", path=iso_path) is None, (
        "isolation dir unexpectedly contains python3.14"
    )
    script = CORPUS / "001_hello.py"
    ref = subprocess.run([reference_or_skip(), str(script)], capture_output=True, cwd=REPO_ROOT, timeout=60)
    app = build_binary(script, "iso_app")
    proc = run_binary(app, path_env=iso_path)
    assert proc.stdout == ref.stdout
    assert proc.stderr == ref.stderr
    assert proc.returncode == ref.returncode


@XFAIL_0206
@pytest.mark.parametrize("stem", EXIT_CASES)
def test_binary_exit_code_contract(stem):
    """C-28/C-29/C-30 measured ON THE BINARY: each inline case, compiled and
    run standalone, reproduces its pinned (stdout, stderr, rc) triple."""
    app = build_binary(CASES / (stem + ".py"), f"{stem}_app")
    compare_case(app, stem)


@XFAIL_0206
@pytest.mark.parametrize("rel", SLICE_30)
def test_ap_conform_slice_against_binary(rel):
    """02.06 Gate verbatim: "conformance suite runs against the BINARY, not
    the JIT, for a 30-case slice" — per case: build, then
    `ap-conform run --corpus <case> --impl <built app>` (00.04 CLI, G-5
    per-case invocation form). Exit 0 == all three channels byte-equal."""
    script = CORPUS / rel
    tag = Path(rel).stem.replace("/", "_") + "_app"
    app = build_binary(script, tag)
    cmd = _ap_conform_base() + ["run", "--corpus", str(script), "--impl", str(app)]
    proc = subprocess.run(cmd, capture_output=True, cwd=REPO_ROOT, timeout=300)
    assert proc.returncode == 0, (
        f"{rel}: ap-conform vs binary failed rc={proc.returncode}\n"
        f"{proc.stdout.decode(errors='replace')}\n{proc.stderr.decode(errors='replace')}"
    )


@XFAIL_0206
def test_ap_conform_whole_slice_against_binaries():
    """Aggregate rollup of the per-case gate: all 30 must pass; failure detail
    lives in the per-case tests above (this one is the CI one-liner)."""
    failures = []
    for rel in SLICE_30:
        script = CORPUS / rel
        app = build_binary(script, Path(rel).stem + "_agg_app")
        cmd = _ap_conform_base() + ["run", "--corpus", str(script), "--impl", str(app)]
        proc = subprocess.run(cmd, capture_output=True, cwd=REPO_ROOT, timeout=300)
        if proc.returncode != 0:
            failures.append(rel)
    assert not failures, f"slice-vs-binary failures: {failures}"


@XFAIL_0206
def test_binary_size_within_bound():
    """02.06 Gate: "binary <= 15MB musl". GAP-2 platform handling: the bound is
    ASSERTED only in linux-musl mode (APX_TARGET=linux-musl); on macOS-arm64 the
    link is "standard for macos", no contractual bound exists, so the size is
    MEASURED AND REPORTED only. The hello binary is the measured artifact."""
    app = build_binary(CORPUS / "001_hello.py", "size_app")
    size = app.stat().st_size
    mode = os.environ.get("APX_TARGET", "macos-arm64")
    print(f"T-09 SIZE REPORT: {mode} hello binary = {size:,} bytes "
          f"({size / (1024 * 1024):.2f} MiB)")
    if mode == "linux-musl":
        assert size <= 15 * 1024 * 1024, f"musl binary {size} > 15MB (02.06 Gate)"
    else:
        assert size > 0  # report-only on macos (GAP-2)


@XFAIL_0402
def test_reproducible_build_byte_identity():
    """CROSS-REF 04.02 (not a 02.06 blocker — GAP-3): two builds of the same
    source into fresh paths MUST be byte-identical once the linker build-id is
    zeroed per the 04.02 reproducibility gate."""
    script = CORPUS / "001_hello.py"
    root = Path(tempfile.mkdtemp(prefix="t09_repro_"))
    a = root / "a" / "app"
    b = root / "b" / "app"
    for out in (a, b):
        out.parent.mkdir(parents=True, exist_ok=True)
        proc = subprocess.run(
            _cli_cmd() + ["build", str(script), "-o", str(out)],
            capture_output=True,
            cwd=REPO_ROOT,
            env=_clean_env(),
            timeout=300,
        )
        assert proc.returncode == 0
        assert out.exists(), "build produced no artifact"
    assert a.read_bytes() == b.read_bytes(), "two builds differ byte-wise (04.02)"


@XFAIL_0206
def test_argv_passthrough():
    """02.06 Expose UX (single static binary): sys.argv inside the binary is
    what is given AFTER the binary name — `./app alpha 2` -> ['alpha', '2'].
    x011's triple was pinned with exactly these args on the reference."""
    app = build_binary(CASES / "x011_argv_tail.py", "argv_app")
    proc = run_binary(app, args=("alpha", "2"))
    exp = (EXPECTED / "x011_argv_tail.stdout").read_bytes()
    assert proc.stdout == exp, f"argv echo drift: {proc.stdout!r} != {exp!r}"
    assert proc.returncode == 0


@XFAIL_0206
def test_cwd_invariance():
    """The same binary run from two different working directories produces
    identical bytes + rc (C-04/C-27; x013 is cwd-invariant by construction —
    a static binary that breaks on cwd is a 02.06 packaging bug)."""
    app = build_binary(CASES / "x013_cwd_probe.py", "cwd_app")
    d1 = Path(tempfile.mkdtemp(prefix="t09_cwd1_"))
    d2 = Path(tempfile.mkdtemp(prefix="t09_cwd2_"))
    p1 = run_binary(app, cwd=d1)
    p2 = run_binary(app, cwd=d2)
    assert (p1.stdout, p1.stderr, p1.returncode) == (p2.stdout, p2.stderr, p2.returncode)
    assert p1.stdout == (EXPECTED / "x013_cwd_probe.stdout").read_bytes()
