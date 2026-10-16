# T-05 — values (gates implementation leaf 02.01 ap-obj core values).
#
# Authorship: written from the spec pack ONLY (docs/spec/01-values-and-objects.md,
# docs/spec/06-observable-behavior.md, docs/compat-contract.md,
# plans/antipythany/master.md, plans/antipythany/tests/README.md, and leaf 02.01's
# Expose/Gate in plans/antipythany/02-native-backend/orchestrator.md). The author
# has not read any implementation source under crates/.
#
# This file has TWO kinds of tests:
#
#  (A) FIXTURE self-checks (must be GREEN today; they test the corpus, not
#      antipythany): every tests/corpus/values/V*.py has an expected
#      .stdout/.stderr/.exit triple; re-running the case under
#      /opt/homebrew/bin/python3.14 reproduces the pinned bytes exactly; all
#      triples exist and stdout is non-empty for every case; the reference-vs-
#      reference differential through `ap-conform run --impl <python3.14>` is
#      green over the whole slice (per S-06-1/S-06-2 the harness compares the
#      three channels byte-exactly; a green self-comparison proves the harness
#      contract holds on this slice).
#
#  (B) ANTIPYTHANY-side gate tests (xfail(strict=False) until leaf 02.01 lands):
#      `ap-conform run --corpus tests/corpus/values/*.py --impl <antipythany>`
#      must go green on the whole slice; plus two per-channel spot gates against
#      the pinned expected triples (stdout bytes, exit code).
#
# See plans/antipythany/tests/T-05-values.md for the case table and citations.

import glob
import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS_DIR = REPO_ROOT / "tests" / "corpus" / "values"
EXPECTED_DIR = CORPUS_DIR / "expected"
REF_PY = "/opt/homebrew/bin/python3.14"
AP_CONFORM = os.environ.get(
    "AP_CONFORM", str(REPO_ROOT / "target" / "debug" / "ap-conform")
)
APX_IMPL = os.environ.get("APX_IMPL", "")  # e.g. "target/debug/antipythany run"

XFAIL_REASON = "awaits 02.01 ap-obj core values (antipythany impl side)"


def cases():
    return sorted(CORPUS_DIR.glob("V*.py"))


ALL_CASES = cases()


def case_ids():
    return [p.stem for p in ALL_CASES]


# ---------------------------------------------------------------------------
# (A) Fixture self-checks — green today.
# ---------------------------------------------------------------------------


def test_corpus_slice_is_populated():
    # Gate shape: leaf 02.01's gate is "ap-conform green on the values corpus
    # slice"; the slice must exist with >= 40 cases covering spec 01.
    assert len(ALL_CASES) >= 40, f"only {len(ALL_CASES)} cases in {CORPUS_DIR}"


@pytest.mark.parametrize("case", ALL_CASES, ids=case_ids())
def test_expected_triple_exists(case):
    stem = case.stem
    for suffix in (".stdout", ".stderr", ".exit"):
        assert (EXPECTED_DIR / (stem + suffix)).is_file(), (
            f"missing expected/{stem}{suffix}"
        )


@pytest.mark.parametrize("case", ALL_CASES, ids=case_ids())
def test_expected_stdout_nonempty_and_exit_pinned(case):
    # Every case prints observable bytes (S-06-3); exit file is one ASCII int.
    out = (EXPECTED_DIR / (case.stem + ".stdout")).read_bytes()
    assert out != b"", f"{case.stem}: empty expected stdout"
    rc = (EXPECTED_DIR / (case.stem + ".exit")).read_bytes()
    assert rc.strip().isdigit() or (rc.strip().lstrip(b"-").isdigit()), (
        f"{case.stem}: bad .exit content {rc!r}"
    )


@pytest.mark.parametrize("case", ALL_CASES, ids=case_ids())
def test_expected_bytes_stable(case):
    # Re-run the reference and byte-compare all three channels (S-06-1: no
    # normalization, no trimming). A failure here means the case is
    # nondeterministic (unmasked UNSPECIFIED behavior leaked into output) and
    # must be FIXED BY MASKING, never by weakening the case (C-33).
    proc = subprocess.run(
        [REF_PY, str(case)], capture_output=True, timeout=120, cwd=str(REPO_ROOT)
    )
    stem = case.stem
    exp_out = (EXPECTED_DIR / (stem + ".stdout")).read_bytes()
    exp_err = (EXPECTED_DIR / (stem + ".stderr")).read_bytes()
    exp_rc = int((EXPECTED_DIR / (stem + ".exit")).read_bytes())
    assert proc.stdout == exp_out, (
        f"{stem}: stdout drift\n got={proc.stdout!r}\nwant={exp_out!r}"
    )
    assert proc.stderr == exp_err, (
        f"{stem}: stderr drift\n got={proc.stderr!r}\nwant={exp_err!r}"
    )
    assert proc.returncode == exp_rc, (
        f"{stem}: exit drift got {proc.returncode} want {exp_rc}"
    )


@pytest.mark.parametrize("case", ALL_CASES, ids=case_ids())
def test_case_header_cites_clauses(case):
    # tests/README.md corpus rule: every case cites spec clause ids.
    text = case.read_text(encoding="utf-8")[:800]
    assert "# Cites:" in text and ("S-01-" in text or "C-" in text), (
        f"{case.stem}: header does not cite clause ids"
    )


def test_no_salted_hash_printed():
    # S-01-38 / S-06-16: str/bytes hash values are salted in the reference and
    # MUST NOT be corpus-anchored. Lint the sources: no `hash("<literal>` or
    # `hash(b"...` result printed. (Deterministic-masking invariant.)
    offenders = []
    for case in ALL_CASES:
        for i, line in enumerate(case.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("#"):
                continue
            if 'hash("' in line or "hash(b'" in line or 'hash(b"' in line:
                if "== hash(" not in line and "!=" not in line:
                    offenders.append(f"{case.stem}:{i}: {line.strip()}")
    assert not offenders, "salted str/bytes hash printed:\n" + "\n".join(offenders)


def test_no_forbidden_apis():
    # C-19: refcount introspection is corpus-prohibited; S-06-17 additionally
    # prohibits id(), __del__ timing, time values, os.getpid in anchors.
    banned = ("getrefcount", "gettotalrefcount", "getallocatedblocks", "id(")
    offenders = []
    for case in ALL_CASES:
        for i, line in enumerate(case.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("#"):
                continue
            for b in banned:
                if b in line:
                    offenders.append(f"{case.stem}:{i}: {line.strip()}")
    assert not offenders, "\n".join(offenders)


@pytest.mark.skipif(not Path(AP_CONFORM).is_file(), reason="ap-conform bin missing")
def test_apconform_self_comparison_green():
    # 00.04 harness contract re-proven on THIS slice: reference vs reference
    # through ap-conform must pass byte-exactly on all three channels.
    proc = subprocess.run(
        [AP_CONFORM, "run", "--corpus", "tests/corpus/values/*.py",
         "--impl", REF_PY, "--json"],
        capture_output=True, text=True, timeout=300, cwd=str(REPO_ROOT),
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr
    import json

    data = json.loads(proc.stdout)
    summary = data["summary"]
    assert summary["failed"] == 0 and summary["errors"] == 0 and summary["timeouts"] == 0, (
        f"harness self-comparison not green: {summary}"
    )
    assert summary["passed"] == summary["total"] == len(ALL_CASES), summary


# ---------------------------------------------------------------------------
# (B) Antipythany-side gates — xfail(strict=False) until 02.01 lands.
# ---------------------------------------------------------------------------

xfail_ap = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)


def _ap_conform_available():
    return Path(AP_CONFORM).is_file()


def _impl_cmd():
    if APX_IMPL:
        import shlex

        return shlex.split(APX_IMPL)
    runner = REPO_ROOT / "target" / "debug" / "antipythany"
    if runner.is_file():
        return [str(runner), "run"]
    return None


@pytest.mark.skipif(not _ap_conform_available(), reason="ap-conform bin missing")
@xfail_ap
def test_apconform_antipythany_full_slice():
    # Leaf 02.01 Gate: "ap-conform green on the values corpus slice".
    impl = _impl_cmd()
    assert impl is not None, "no antipythany runner; set APX_IMPL"
    proc = subprocess.run(
        [AP_CONFORM, "run", "--corpus", "tests/corpus/values/*.py",
         "--impl", *impl, "--timeout", "60"],
        capture_output=True, text=True, timeout=1800, cwd=str(REPO_ROOT),
    )
    assert proc.returncode == 0, proc.stdout[-4000:] + proc.stderr[-2000:]


@pytest.mark.skipif(not _ap_conform_available(), reason="ap-conform bin missing")
@xfail_ap
@pytest.mark.parametrize(
    "stem",
    ["V001-bigint-exactness", "V061-float-repr-shortest",
     "V040-dict-insertion-order", "V047-truthiness-table",
     "V055-hash-eq-invariant", "V065-container-repr-cycle"],
)
def test_apconform_spot_cases_against_pinned_bytes(stem):
    # Per-channel gate (C-24/S-06-1): run ONE case under the impl and diff the
    # three channels against the pinned expected triples. Spot set covers
    # bigint exactness, float shortest repr, dict order, truthiness, hash
    # invariant, container repr+cycle.
    impl = _impl_cmd()
    assert impl is not None, "no antipythany runner; set APX_IMPL"
    case = CORPUS_DIR / (stem + ".py")
    proc = subprocess.run([*impl, str(case)], capture_output=True, timeout=120,
                          cwd=str(REPO_ROOT))
    assert proc.stdout == (EXPECTED_DIR / (stem + ".stdout")).read_bytes(), stem
    # stderr paths inside tracebacks may legitimately differ per harness
    # convention only when non-empty; on this slice every case has empty stderr.
    assert proc.stderr == (EXPECTED_DIR / (stem + ".stderr")).read_bytes(), stem
    assert proc.returncode == int(
        (EXPECTED_DIR / (stem + ".exit")).read_bytes()
    ), stem
