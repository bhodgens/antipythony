# T-06 — classes + exception-hierarchy test suite (gates leaf 02.03 ap-obj).
#
# Authorship: written from the spec pack ONLY (docs/spec/03-classes.md,
# docs/spec/02-control-flow.md, docs/spec/06-observable-behavior.md,
# docs/compat-contract.md, plans/antipythony/master.md,
# plans/antipythony/02-native-backend/orchestrator.md 02.03 Expose/Gate).
# No implementation source under crates/ was read. Per-case citations and
# CONTRACT-GAPS: plans/antipythony/tests/T-06-classes.md.
#
# The suite has two halves:
#  1. FIXTURE RE-VERIFICATION (green today): every corpus script under
#     tests/corpus/classes/ and tests/corpus/exceptions/ is re-run with the
#     reference CPython and its stdout/stderr/exit bytes are diffed against
#     the recorded expected/ files; headers are checked for clause citations;
#     nondeterministic-address leakage is checked (S-06-17 #4, C-24).
#  2. ap-conform GATE (xfail, strict=False): the 02.03 gate — "corpus
#     classes/exceptions slices green under tier-0" — is expressed as
#     `ap-conform run --corpus <slice> --impl <antipythony> --tier 0`.
#     Neither the ap-conform binary seam nor the ap-obj implementation exists
#     yet, so these XFAIL until 02.03 lands.

import os
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]


def _mask_root(data: bytes) -> bytes:
    """Normalize absolute corpus paths in stderr self-check pins.

    CPython 3.14 resolves the script path to an ABSOLUTE path in tracebacks
    and warning locations, so pins recorded the original checkout's absolute
    path and failed in every other worktree/CI. The mask collapses any
    absolute prefix that precedes a tests/corpus/ path to <ROOT>/... .
    Line numbers, caret columns, and message text are untouched, so the pin
    still diffs everything semantic (C-33: mask, never weaken). Follows the
    in-tree precedent tests/t09_binary (_norm_stderr, S-06-7 caveat). The
    differential gate (ap-conform, reference vs implementation) stays
    byte-exact with NO masking; this applies only to reference self-checks.
    """
    return re.sub(rb"/[^\s\"',;()]*?(tests/corpus/[^\s\"',;()]*)",
                  rb"<ROOT>/\1", data)
SPEC03 = REPO_ROOT / "docs" / "spec" / "03-classes.md"
CORPUS = REPO_ROOT / "tests" / "corpus"
SLICE_DIRS = {
    "classes": CORPUS / "classes",
    "exceptions": CORPUS / "exceptions",
}

# Reference CPython (C-01 caveat G-7: recorded from 3.14.7, provisional until
# re-verified on pinned 3.14.0 — same open item as the compat contract).
# Reference discovery: unified env+PATH ladder (robustness-ci 01.01); see
# tests/tools/_ref_python.py for the AP_REQUIRE_REFERENCE=1 policy.
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
from _ref_python import reference_or_skip  # noqa: E402

XFAIL_REASON = "awaits 02.03 ap-obj + ap-conform --impl seam"


def _cases():
    out = []
    for sname in sorted(SLICE_DIRS):
        for p in sorted(SLICE_DIRS[sname].glob("*.py")):
            out.append((sname, p))
    return out


CASES = _cases()
IDS = [f"{s}/{p.stem}" for s, p in CASES]


def _expected_dir(script: Path) -> Path:
    return script.parent / "expected"


def _run_ref(script: Path) -> subprocess.CompletedProcess:
    """Run a corpus case as a FILE under the reference, same convention as
    ap-conform (spec 02 header: identical argv both impls; spec 06 S-06-2:
    separate streams, scrubbed env)."""
    env = {k: os.environ[k] for k in ("PATH", "HOME", "LANG") if k in os.environ}
    return subprocess.run(
        [reference_or_skip(), str(script)],
        capture_output=True,
        env=env,
        cwd=REPO_ROOT,
        timeout=60,
    )


def _spec_clause_ids():
    text = SPEC03.read_text()
    return {int(m) for m in re.findall(r"\*\*S-03-(\d{2})\b", text)}


def _cited_ids(script: Path):
    first = script.read_text().splitlines()[0]
    assert first.startswith("# Cites:"), (
        f"{script.name}: first line must be a '# Cites: S-03-nn…' header "
        f"(tests/README.md corpus rules); got: {first!r}"
    )
    return {int(m) for m in re.findall(r"S-03-(\d{2})", first)}


def _ap_conform_base() -> list[str]:
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


def _impl_cmd() -> list[str]:
    override = os.environ.get("APX_IMPL")
    if override:
        return shlex.split(override)
    found = shutil.which("antipythony")
    if found:
        return [found, "run"]
    raise FileNotFoundError(
        "antipythony impl command not found (set APX_IMPL=<cmdline> to override)"
    )


def run_conform(corpus_arg: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        _ap_conform_base()
        + ["run", "--corpus", corpus_arg, "--impl", " ".join(_impl_cmd()), "--tier", "0"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=600,
    )


xfail_ap_conform = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)

# ---------------------------------------------------------------------------
# Half 1 — fixture re-verification (GREEN today, no implementation needed)
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("case", IDS)
def test_header_cites_existing_clauses(case):
    sname, stem = case.split("/")
    script = SLICE_DIRS[sname] / (stem + ".py")
    cited = _cited_ids(script)
    assert cited, f"{case}: no S-03-nn ids in the header"
    spec_ids = _spec_clause_ids()
    unknown = cited - spec_ids
    assert not unknown, f"{case}: cites ids not present in spec 03: {sorted(unknown)}"


def test_every_spec03_must_clause_is_covered():
    # Corpus-traceability (tests/README.md): every S-03 MUST clause gets at
    # least one differential case.
    cited = set()
    for _, script in CASES:
        cited |= _cited_ids(script)
    spec_ids = _spec_clause_ids()
    missing = spec_ids - cited
    assert not missing, f"clauses with no corpus case: {sorted(missing)}"
    assert len(spec_ids) == 34, f"expected 34 S-03 clauses, found {len(spec_ids)}"


def test_slices_meet_quotas():
    # Task contract: ~30 class cases, ~15 exception cases. Corpus only grows
    # (C-33) — assert the FLOOR, never an exact count.
    n_classes = sum(1 for s, _ in CASES if s == "classes")
    n_exc = sum(1 for s, _ in CASES if s == "exceptions")
    assert n_classes >= 30, f"classes slice has {n_classes}"
    assert n_exc >= 15, f"exceptions slice has {n_exc}"


@pytest.mark.parametrize("case", IDS)
def test_reference_reproduces_expected_bytes(case):
    sname, stem = case.split("/")
    script = SLICE_DIRS[sname] / (stem + ".py")
    exp = _expected_dir(script)
    proc = _run_ref(script)
    out = (exp / (stem + ".out")).read_bytes()
    err = (exp / (stem + ".err")).read_bytes()
    rc = (exp / (stem + ".rc")).read_text().strip()
    assert proc.stdout == out, (
        f"{case}: stdout drift\n--- got ---\n{proc.stdout.decode(errors='replace')}"
        f"\n--- expected ---\n{out.decode(errors='replace')}"
    )
    assert _mask_root(proc.stderr) == _mask_root(err), (
        f"{case}: stderr drift\n{proc.stderr.decode(errors='replace')}"
    )
    assert str(proc.returncode) == rc, f"{case}: exit code {proc.returncode} != {rc}"


@pytest.mark.parametrize("case", IDS)
def test_expected_streams_are_clean(case):
    # Every case swallows its expected exceptions in-script: rc 0 and empty
    # stderr by construction, so traceback path text never enters an expected
    # file (spec 02 path-normalization caveat is moot for this slice).
    sname, stem = case.split("/")
    exp = _expected_dir(SLICE_DIRS[sname] / (stem + ".py"))
    assert (exp / (stem + ".rc")).read_text().strip() == "0", case
    assert (exp / (stem + ".err")).read_bytes() == b"", case
    assert (exp / (stem + ".out")).read_bytes().endswith(b"\n"), case


@pytest.mark.parametrize("case", IDS)
def test_no_nondeterministic_addresses_leak(case):
    # S-06-17 #4 / C-24: memory addresses are never diff-able. Masking
    # decisions must have kept every `0x…` out of the expected bytes.
    sname, stem = case.split("/")
    exp = _expected_dir(SLICE_DIRS[sname] / (stem + ".py"))
    blob = (exp / (stem + ".out")).read_bytes() + (exp / (stem + ".err")).read_bytes()
    m = re.search(rb"0x[0-9a-fA-F]{4,}", blob)
    assert m is None, f"{case}: address-like token {m.group()!r} in expected bytes"


# ---------------------------------------------------------------------------
# Half 2 — ap-conform differential gate (xfail until 02.03 lands)
# ---------------------------------------------------------------------------


@xfail_ap_conform
@pytest.mark.parametrize("sname", sorted(SLICE_DIRS))
def test_ap_conform_slice_green_tier0(sname):
    # 02.03 Gate: "corpus classes/exceptions slices green under tier-0".
    proc = run_conform(str(SLICE_DIRS[sname] / "*.py"))
    assert proc.returncode == 0, (
        f"{sname} slice not green:\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
    )


@xfail_ap_conform
@pytest.mark.parametrize("case", IDS)
def test_ap_conform_case_green_tier0(case):
    sname, stem = case.split("/")
    script = SLICE_DIRS[sname] / (stem + ".py")
    proc = run_conform(str(script))
    assert proc.returncode == 0, (
        f"{case} not green:\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
    )
