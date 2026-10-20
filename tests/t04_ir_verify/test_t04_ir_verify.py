"""T-04 IR-verify differential suite (gates leaf 01.04, ap_ir::verify).

Authored from the spec pack ONLY (docs/spec/02-control-flow.md,
docs/spec/01-values-and-objects.md S-01-31, docs/compat-contract.md,
plans/antipythany/tests/T-04-ir-verify.md) by an agent that never read crate
sources. See the T-04 md for clause citations and CONTRACT-GAPS.

Two halves:

1. FIXTURE SIDE (passes today): fixtures/expected_t04.json pins, verbatim, what
   the reference interpreter (/opt/homebrew/bin/python3.14, 3.14.7) produced for
   every probe in probes/ — one execution per probe via
   `/opt/homebrew/bin/python3.14 <abs-path>` (regenerate with
   /opt/homebrew/bin/python3.14 tests/t04_ir_verify/gen_expectations.py).
   These tests check the expect side is complete, version-pinned, internally
   consistent, and carries the exact S-02-2 message template and S-02-37
   caret-marker format.

2. SEAM SIDE (01.04's CLI has landed; formerly xfail): the bin
   `ap-verify-dump <file>` prints one line per diagnostic,
   `<path>:<line>:<col> <class>: <message>` (C-10 machine-stable shape,
   T-04 doc G-1), matching the pinned CPython fields for every flagged probe,
   and prints NOTHING for every silence probe. Resolution order: env
   AP_VERIFY_DUMP, PATH, target/debug, target/release; absent -> stub exit 127
   -> pytest.xfail with the leaf-01.04 reason (missing-bin safety net only).

UnboundLocalError clause citation for every flagged probe lives in
plans/antipythany/tests/T-04-ir-verify.md's per-test table; probes themselves
carry header comments.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SUITE = Path(__file__).resolve().parent
PROBES = SUITE / "probes"
FIXTURE_PATH = SUITE / "fixtures" / "expected_t04.json"

XFAIL_REASON = "awaits 01.04 ap-verify-dump seam (ap_ir::verify is a lib fn; T-04 doc G-1)"

# Reference interpreter path (robustness-ci 01.01): unified env+PATH ladder
# via tests/tools/_ref_python.py. Resolved lazily; None when absent (this
# suite currently only records the constant for docs — no spawns).
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests" / "tools"))
from _ref_python import discover_reference  # noqa: E402

PYTHON_REFERENCE = discover_reference() or "/opt/homebrew/bin/python3.14"

# The exact 3.14 message template pinned by S-02-2 (NOT the pre-3.11 wording).
UNBOUND_LOCAL_TEMPLATE = (
    "cannot access local variable {name!r} where it is not associated with a value"
)
# Free-variable variant observed on 3.14.7 (T-04 doc G-3; p20).
FREE_VARIABLE_TEMPLATE = (
    "cannot access free variable {name!r} where it is not associated with a "
    "value in enclosing scope"
)

# ---------------------------------------------------------------------------
# probe classification (mirrors the T-04 doc tables exactly)
# ---------------------------------------------------------------------------

# verify MUST flag: static unbound-LOCAL reads (S-02-2 family)
RUNTIME_FLAGGED = [
    "p01_unbound_basic.py",
    "p02_unbound_branch.py",
    "p03_unbound_while_body.py",
    "p04_aug_unbound.py",
    "p05_for_empty_iter.py",
    "p06_comprehension_scope_unbound.py",
    "p07_del_unbound_local.py",
    "p08t_try_read_before_uncaught.py",
]
# verify MUST flag: compile-time static checks (S-02-3/4/11/18, S-04-10, S-02-8)
COMPILE_FLAGGED = [
    "p09_global_nonlocal.py",
    "p10_nonlocal_no_binding.py",
    "p11_nonlocal_at_module.py",
    "p12_break_outside.py",
    "p13_continue_outside.py",
    "p14_return_outside.py",
    "p15_import_star_in_fn.py",
    "p16_star_in_class.py",
    "p17_invalid_default.py",
    "p18_dup_param.py",
]
# S-02-2 is a STATIC rule: p08 runs clean at runtime (except catches the
# subclass) yet verify must still flag the unbound read.
MUST_FLAG_DESPITE_CLEAN = ["p08_try_read_before_caught.py"]
# Boundary set (T-04 doc G-3): real runtime errors outside the unbound-LOCAL
# requirement. If verify reports them, class+message must match CPython;
# absence is acceptable.
BOUNDARY_OPTIONAL = [
    "p19_del_then_read_module.py",
    "p20_closure_read_after_return.py",
    "p21_comprehension_no_leak.py",
    "p22_class_shadowing.py",
]
# verify MUST be silent (CPython runs clean; S-01-31 n09 warns but has no
# unbound name).
SILENCE = [
    "n01_closure_after_return.py",
    "n02_closure_cell_set_after_def.py",
    "n03_for_loop_var.py",
    "n04_try_except_assign.py",
    "n05_module_global_read.py",
    "n06_class_body_valid.py",
    "n07_lambda_late_binding.py",
    "n08_comprehension_first_iterable.py",
    "n09_identity_no_local_error.py",
]
ALL_FLAGGED = RUNTIME_FLAGGED + COMPILE_FLAGGED + MUST_FLAG_DESPITE_CLEAN

DIAG_RE = re.compile(r"^(?P<path>.+):(?P<line>\d+):(?P<col>\d+) (?P<cls>[\w.]+): (?P<msg>.+)$")


def load_fixture() -> dict:
    return json.loads(FIXTURE_PATH.read_text())


# ---------------------------------------------------------------------------
# seam runner (same discovery pattern as T-01/T-02)
# ---------------------------------------------------------------------------

def _find_ap_verify_dump() -> Path | None:
    env = os.environ.get("AP_VERIFY_DUMP")
    if env and Path(env).is_file():
        return Path(env)
    on_path = shutil.which("ap-verify-dump")
    if on_path:
        return Path(on_path)
    for profile in ("debug", "release"):
        cand = REPO / "target" / profile / "ap-verify-dump"
        if cand.is_file():
            return cand
    return None


class _StubRunner:
    """Stand-in for the not-yet-built bin: a stub script that exits 127."""

    def __init__(self) -> None:
        self.stub = Path(
            os.environ.get(
                "T04_STUB",
                Path(__file__).with_name("_ap_verify_dump_stub.sh").as_posix(),
            )
        )
        if not self.stub.exists():
            self.stub.write_text("#!/bin/sh\nexit 127\n")
            self.stub.chmod(0o755)

    def run(self, args: list[str]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(self.stub), *args], capture_output=True, text=True,
            timeout=60, check=False,
        )


@pytest.fixture(scope="session")
def ap_verify_dump():
    real = _find_ap_verify_dump()
    return real if real is not None else _StubRunner()


def run_seam(runner, probe_name: str) -> subprocess.CompletedProcess[str]:
    if isinstance(runner, Path):
        return subprocess.run(
            [str(runner), str(PROBES / probe_name)],
            capture_output=True, text=True, timeout=120, check=False, cwd=REPO,
        )
    return runner.run([str(PROBES / probe_name)])


def require_seam(proc: subprocess.CompletedProcess[str]) -> None:
    """Stub (exit 127, no bin) => xfail with the leaf reason, not an error."""
    if proc.returncode == 127 and proc.stdout == "" and proc.stderr == "":
        pytest.xfail(XFAIL_REASON)


def parse_diagnostics(stdout: str) -> list[dict]:
    diags = []
    for ln in stdout.splitlines():
        if not ln.strip():
            continue
        m = DIAG_RE.match(ln)
        assert m is not None, f"unparseable diagnostic line (C-10 shape violated): {ln!r}"
        diags.append(m.groupdict())
    return diags


def assert_matches(diags: list[dict], rec: dict, probe_name: str) -> None:
    """At least one diagnostic matches the pinned class+message+line (+col when pinned).

    Diagnostic paths must name the probe file (C-08 requires the offending
    location; compare by basename so absolute/relative spellings both pass).
    """
    want_cls = rec["error_class"]
    matched = [
        d for d in diags
        if d["cls"].split(".")[-1] == want_cls
        and d["msg"] == rec["message"]
        and int(d["line"]) == rec["expected_line"]
        and (rec["expected_col"] is None or int(d["col"]) == rec["expected_col"])
        and Path(d["path"]).name == probe_name
    ]
    assert matched, (
        f"{probe_name}: verify must emit a diagnostic matching pinned CPython "
        f"report class={want_cls!r} msg={rec['message']!r} "
        f"line={rec['expected_line']} col={rec['expected_col']}; got:\n"
        + "\n".join(f"  {d['cls']}: {d['msg']} @ {d['line']}:{d['col']}" for d in diags)
    )


# ===========================================================================
# 1. Fixture-side integrity (real today — the expect half of the differential)
# ===========================================================================

def test_fixture_covers_every_probe():
    """Every probe has exactly one pinned record and every record has a probe."""
    fx = load_fixture()["probes"]
    on_disk = sorted(p.name for p in PROBES.glob("*.py"))
    assert sorted(fx) == on_disk, "fixture set drifted from probes/ set"
    # classification sets must partition the corpus (no probe unclassified)
    classified = set(ALL_FLAGGED + BOUNDARY_OPTIONAL + SILENCE)
    assert classified == set(on_disk), (
        f"unc probes: {set(on_disk) - classified}; stale: {classified - set(on_disk)}"
    )


def test_fixture_reference_is_pinned_3147():
    """Expect side is only valid if produced by the reference 3.14.7 binary."""
    meta = load_fixture()
    assert meta["python_version"].startswith("3.14.7"), (
        "fixtures must be regenerated with /opt/homebrew/bin/python3.14 (3.14.7)"
    )
    assert meta["python_executable"].endswith("python3.14")
    # C-01 version-probe spirit: the pinned run's interpreter is the contract target.


def test_fixture_derived_fields_consistent():
    """Stored error_class/message/line/col equal re-derivation from stored stderr.

    Determinism of the expect side (C-10 machine-stability principle): the
    fields the seam tests assert against are pure functions of the pinned
    stderr bytes, recomputed here with the generator's own derive().
    """
    sys.path.insert(0, str(SUITE))
    import gen_expectations  # noqa: E402

    fx = load_fixture()["probes"]
    for name, rec in fx.items():
        rederived = gen_expectations.derive(
            Path("<PROBE>"), rec["rc"], rec["stdout"], rec["stderr"]
        )
        for field in ("kind", "error_class", "message", "expected_line", "expected_col"):
            assert rederived[field] == rec[field], (name, field)


def test_fixture_kinds_are_known():
    kinds = {rec["kind"] for rec in load_fixture()["probes"].values()}
    assert kinds <= {"clean", "clean-with-warning", "runtime-traceback",
                     "compile-syntaxerror"}, kinds


# ===========================================================================
# 2. Pinned exact message text (S-02-2 wording, quoted verbatim from 3.14.7)
# ===========================================================================

@pytest.mark.parametrize(
    "probe_name,var",
    [("p01_unbound_basic.py", "z"), ("p02_unbound_branch.py", "y"),
     ("p04_aug_unbound.py", "x"), ("p07_del_unbound_local.py", "q")],
    ids=["read", "branch", "aug", "del-local"],
)
def test_pinned_message_exact(probe_name, var):
    """The pinned stderr carries the exact S-02-2 3.14 message template.

    'cannot access local variable ... where it is not associated with a value'
    with the name repr-quoted — and NOT the pre-3.11
    'referenced before assignment' wording (S-02-2).
    """
    rec = load_fixture()["probes"][probe_name]
    exact = UNBOUND_LOCAL_TEMPLATE.format(name=var)
    assert rec["error_class"] == "UnboundLocalError", probe_name
    assert rec["message"] == exact, (probe_name, rec["message"])
    assert exact in rec["stderr"]
    assert "referenced before assignment" not in rec["stderr"]


def test_pinned_free_variable_variant():
    """p20 pins the DIFFERENT free-variable message + NameError class (G-3)."""
    rec = load_fixture()["probes"]["p20_closure_read_after_return.py"]
    assert rec["error_class"] == "NameError"
    assert rec["message"] == FREE_VARIABLE_TEMPLATE.format(name="v")
    assert rec["message"] in rec["stderr"]


# ===========================================================================
# 3. FGE caret-marker format pinned from real runs (S-02-37)
# ===========================================================================

@pytest.mark.parametrize(
    "probe_name",
    ["p01_unbound_basic.py", "p04_aug_unbound.py",
     "p06_comprehension_scope_unbound.py", "p08t_try_read_before_uncaught.py",
     "p19_del_then_read_module.py", "p20_closure_read_after_return.py"],
    ids=["read-name", "aug-stmt", "comprehension", "try-uncaught", "del-module", "cell"],
)
def test_fge_marker_format(probe_name):
    """Pinned unbound-read tracebacks carry the S-02-37 marker structure:
    indented source line (4 spaces) then a marker line of ~ ^ and spaces with
    at least one '^', aligned under the read site (first '^' == expected_col).
    """
    rec = load_fixture()["probes"][probe_name]
    lines = rec["stderr"].rstrip("\n").splitlines()
    fi = [i for i, ln in enumerate(lines) if ln.startswith('  File "')][-1]
    src = lines[fi + 1]
    marker = lines[fi + 2]
    assert src.startswith("    "), f"{probe_name}: source line not 4-space indented"
    assert marker[4:].strip() and set(marker.strip()) <= {"~", "^"}
    assert "^" in marker
    assert marker.index("^") + 1 == rec["expected_col"]
    # caret sits under the identifier being read (name-only anchor, S-02-37)
    stmt = src.strip()
    ident_start = marker.index("^") - 4
    if rec["message"].startswith("cannot access local variable"):
        mvar = re.search(r"variable '(\w+)'", rec["message"])
        assert mvar is not None, probe_name
        var = mvar.group(1)
        window = stmt[ident_start:ident_start + len(var)]
        assert window == var, f"{probe_name}: caret not under {var!r} in {stmt!r}"


# ===========================================================================
# 4. Seam side — flagged probes (01.04 landed; xfail envelope removed)
# ===========================================================================

@pytest.mark.parametrize("probe_name", RUNTIME_FLAGGED, ids=RUNTIME_FLAGGED)
def test_diff_flagged_runtime(ap_verify_dump, probe_name):
    """S-02-2 differential: verify predicts the pinned UnboundLocalError
    (class, exact message text, read-site line; caret column per G-2)."""
    rec = load_fixture()["probes"][probe_name]
    assert rec["kind"] == "runtime-traceback" and rec["error_class"] == "UnboundLocalError"
    proc = run_seam(ap_verify_dump, probe_name)
    require_seam(proc)
    assert_matches(parse_diagnostics(proc.stdout), rec, probe_name)


@pytest.mark.parametrize("probe_name", COMPILE_FLAGGED, ids=COMPILE_FLAGGED)
def test_diff_flagged_compile(ap_verify_dump, probe_name):
    """Static-check differential: the pinned compile-time SyntaxError
    (S-02-3/4, S-02-11, S-02-18, S-04-10, S-02-8) surfaces as a matching
    verify diagnostic."""
    rec = load_fixture()["probes"][probe_name]
    assert rec["kind"] == "compile-syntaxerror" and rec["error_class"] == "SyntaxError"
    proc = run_seam(ap_verify_dump, probe_name)
    require_seam(proc)
    assert_matches(parse_diagnostics(proc.stdout), rec, probe_name)


def test_diff_must_flag_caught_read(ap_verify_dump):
    """p08: the read is unbound per the STATIC rule; CPython swallows it at
    runtime only because UnboundLocalError subclasses NameError and the except
    clause matches (S-02-2 'determined statically, not dynamically'). The gate
    requires verify to PREDICT the error even when the corpus program never
    crashes — so verify must flag despite the clean run."""
    rec = load_fixture()["probes"]["p08_try_read_before_caught.py"]
    assert rec["rc"] == 0 and rec["stderr"] == "", "pinned premise: runtime-clean"
    proc = run_seam(ap_verify_dump, "p08_try_read_before_caught.py")
    require_seam(proc)
    diags = parse_diagnostics(proc.stdout)
    # the read of x on line 3 is unbound locally (z assigned on line 5 -> local)
    hits = [d for d in diags
            if d["msg"] == UNBOUND_LOCAL_TEMPLATE.format(name="x")
            and int(d["line"]) == 3]
    assert hits, (
        "verify must statically flag the unbound read even though CPython "
        f"swallowed it at runtime; got:\n{proc.stdout!r}"
    )


@pytest.mark.parametrize("probe_name", BOUNDARY_OPTIONAL, ids=BOUNDARY_OPTIONAL)
def test_diff_boundary_optional(ap_verify_dump, probe_name):
    """G-3: beyond the required unbound-LOCAL set. If verify reports these
    runtime NameError variants, class+message must byte-match the pinned
    CPython report; reporting nothing is also allowed (pass below)."""
    rec = load_fixture()["probes"][probe_name]
    proc = run_seam(ap_verify_dump, probe_name)
    require_seam(proc)
    diags = parse_diagnostics(proc.stdout)
    if not diags:
        pytest.skip("boundary set optional per T-04 doc G-3; verify stayed silent")
    assert_matches(diags, rec, probe_name)


# ===========================================================================
# 5. Seam side — silence probes (verify MUST emit nothing)
# ===========================================================================

@pytest.mark.parametrize("probe_name", SILENCE, ids=SILENCE)
def test_diff_silent(ap_verify_dump, probe_name):
    """Clean programs (pinned rc 0; n09 carries only a SyntaxWarning per
    S-01-31, which is not an unbound-name diagnostic): verify emits no
    diagnostic lines (citations per probe in the T-04 doc table)."""
    rec = load_fixture()["probes"][probe_name]
    assert rec["rc"] == 0, f"pinned premise: {probe_name} must run clean under 3.14.7"
    proc = run_seam(ap_verify_dump, probe_name)
    require_seam(proc)
    diags = parse_diagnostics(proc.stdout)
    assert diags == [], (
        f"verify must be silent for {probe_name}; got:\n"
        + "\n".join(f"  {d['cls']}: {d['msg']} @ {d['line']}:{d['col']}" for d in diags)
    )
