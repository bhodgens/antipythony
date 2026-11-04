# T-05 — values (gates implementation leaf 02.01 ap-obj core values).
#
# Authorship: written from the spec pack ONLY (docs/spec/01-values-and-objects.md,
# docs/spec/06-observable-behavior.md, docs/compat-contract.md,
# plans/antipythony/master.md, plans/antipythony/tests/README.md, and leaf 02.01's
# Expose/Gate in plans/antipythony/02-native-backend/orchestrator.md). The author
# has not read any implementation source under crates/.
#
# This file has TWO kinds of tests:
#
#  (A) FIXTURE self-checks (must be GREEN today; they test the corpus, not
#      antipythony): every tests/corpus/values/V*.py has an expected
#      .stdout/.stderr/.exit triple; re-running the case under
#      /opt/homebrew/bin/python3.14 reproduces the pinned bytes exactly; all
#      triples exist and stdout is non-empty for every case; the reference-vs-
#      reference differential through `ap-conform run --impl <python3.14>` is
#      green over the whole slice (per S-06-1/S-06-2 the harness compares the
#      three channels byte-exactly; a green self-comparison proves the harness
#      contract holds on this slice).
#
#  (B) ANTIPYTHONY-side gate tests (xfail(strict=False) until leaf 02.01 lands):
#      `ap-conform run --corpus tests/corpus/values/*.py --impl <antipythony>`
#      must go green on the whole slice; plus two per-channel spot gates against
#      the pinned expected triples (stdout bytes, exit code).
#
# See plans/antipythony/tests/T-05-values.md for the case table and citations.

import glob
import os
import re
import shutil
import subprocess
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
CORPUS_DIR = REPO_ROOT / "tests" / "corpus" / "values"
EXPECTED_DIR = CORPUS_DIR / "expected"

# Reference discovery: unified env+PATH ladder (robustness-ci 01.01, closes
# 00.02's pytest-side gap). reference_or_skip() applies the version-string
# check and the AP_REQUIRE_REFERENCE=1 skip->fail policy; see
# tests/tools/_ref_python.py.
import sys  # noqa: E402
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
from _ref_python import reference_or_skip  # noqa: E402


def _pinned_env() -> dict:
    """S-06-15 scrubbed environment + pinned locale (C-27).

    Only PATH/HOME pass through, with LANG=C. LC_* is deliberately excluded:
    CPython's UTF-8-mode auto-detection rewrites LC_CTYPE in its own
    environment block, so a Python parent leaks it to child processes and
    silently changes their locale. LANG=C is the locale the corpus triples
    were pinned under (PEP 540 auto-enables UTF-8 mode for the C locale).
    """
    env = {k: os.environ[k] for k in ("PATH", "HOME") if k in os.environ}
    env["LANG"] = "C"
    return env


AP_CONFORM = os.environ.get(
    "AP_CONFORM", str(REPO_ROOT / "target" / "debug" / "ap-conform")
)
APX_IMPL = os.environ.get("APX_IMPL", "")  # e.g. "target/debug/antipythony run"

XFAIL_REASON = "awaits 02.01 ap-obj core values (antipythony impl side)"


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
    #
    # Pinned environment (S-06-15 + C-27 "pinned locale environment"): only
    # PATH/HOME plus LANG=C. LC_* must NOT be inherited: CPython's UTF-8-mode
    # auto-detection rewrites LC_CTYPE in its own environment block, so a
    # Python parent leaks it to children and silently flips their locale
    # (io.text_encoding(None) -> 'locale' instead of 'utf-8'). The corpus
    # triples were pinned under this scrub.
    #
    # The case path is passed RELATIVE to REPO_ROOT (cwd is REPO_ROOT) so
    # tracebacks embed a checkout-independent path; an absolute path made the
    # pins fail in every worktree/CI checkout other than the original.
    ref = reference_or_skip()
    proc = subprocess.run(
        [ref, os.path.relpath(case, REPO_ROOT)],
        capture_output=True, timeout=120, cwd=str(REPO_ROOT), env=_pinned_env(),
    )
    stem = case.stem
    exp_out = (EXPECTED_DIR / (stem + ".stdout")).read_bytes()
    exp_err = (EXPECTED_DIR / (stem + ".stderr")).read_bytes()
    exp_rc = int((EXPECTED_DIR / (stem + ".exit")).read_bytes())
    assert proc.stdout == exp_out, (
        f"{stem}: stdout drift\n got={proc.stdout!r}\nwant={exp_out!r}"
    )
    assert _mask_root(proc.stderr) == _mask_root(exp_err), (
        f"{stem}: stderr drift\n got={_mask_root(proc.stderr)!r}\nwant={_mask_root(exp_err)!r}"
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
         "--impl", reference_or_skip(), "--json"],
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
# (B) Antipythony-side gates — xfail(strict=False) until 02.01 lands.
# ---------------------------------------------------------------------------

xfail_ap = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)


def _ap_conform_available():
    return Path(AP_CONFORM).is_file()


def _impl_cmd():
    if APX_IMPL:
        import shlex

        return shlex.split(APX_IMPL)
    runner = REPO_ROOT / "target" / "debug" / "antipythony"
    if runner.is_file():
        return [str(runner), "run"]
    return None


@pytest.mark.skipif(not _ap_conform_available(), reason="ap-conform bin missing")
@xfail_ap
def test_apconform_antipythony_full_slice():
    # Leaf 02.01 Gate: "ap-conform green on the values corpus slice".
    impl = _impl_cmd()
    assert impl is not None, "no antipythony runner; set APX_IMPL"
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
    assert impl is not None, "no antipythony runner; set APX_IMPL"
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
