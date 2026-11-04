# T-08 — tier-1 transparency (gates implementation leaf 02.05 ap-codegen-native tier-1).
#
# Authorship: written from the contract + spec pack ONLY (docs/compat-contract.md
# C-31 tier-invisibility rule + C-29 exit-code/stream invariants + C-24/C-27
# channel contract + C-28, docs/spec/01-values-and-objects.md,
# docs/spec/02-control-flow.md, docs/spec/03-classes.md, plans/antipythony/master.md,
# plans/antipythony/tests/README.md, and leaf 02.05's Expose/Gate in
# plans/antipythony/02-native-backend/orchestrator.md). The author has NOT read any
# implementation source under crates/.
#
# T-08 tests TRANSPARENCY, not speed. Two kinds of tests:
#
#  (A) FIXTURE RE-VERIFICATION (green today; they test the corpus, not
#      antipythony): every program under tests/corpus/tier1-transparency/ (probes)
#      and .../bench/ (speed-slice) has an expected .stdout/.stderr/.exit triple
#      recorded from REAL python3.14 runs (run twice, byte-stable); re-running the
#      program under the reference reproduces the pinned bytes; headers carry
#      clause citations (S-01-nn, S-02-nn, S-03-nn, C-29/C-31) that exist in the
#      cited documents; probes stay within the 10k–100k iteration budget; probes
#      use no timing/identity/refcount APIs (C-24, C-19, D3).
#
#  (B) ANTIPYTHONY-SIDE GATES (xfail(strict=False) until leaf 02.05 lands):
#      1. Cross-mode byte-identity over the ENTIRE tests/corpus tree (globbed at
#         run time — corpus only grows, C-33): each script run under
#         `antipythony run --tier 0`, `antipythony run --tier adaptive`, and
#         `antipythony run --mode binary` must produce byte-identical stdout,
#         byte-identical stderr, and identical exit code (C-31 + C-29 + C-24/C-27).
#      2. Specialization-probe differential: each tier1-transparency probe under
#         each mode must match its pinned reference triple exactly — wrong deopt
#         shows up as corrupted mid-state output.
#      3. The 02.05 adaptive-mode full-corpus gate ("adaptive-mode full corpus
#         green") and the OSR side-effect-exactly-once spot from C-31's Verify.
#      4. Speed-gate STUB (measured, not assumed): >=1.5x geomean over tier-0 on
#         the arithmetic/string benchmark slice; emits a JSON report. Timing is
#         NOT a conformance channel (C-24), so this is a report-and-threshold
#         harness, xfail now, documented in CONTRACT-GAPS of
#         plans/antipythony/tests/T-08-tier1-transparency.md.
#
# CLI seam: no doc read by this author pins the tier/mode flag vocabulary
# (00.04 ap-conform exposes --corpus/--impl/--filter/--json only). The suite
# assumes `antipythony run --tier 0` / `antipythony run --tier adaptive` /
# `antipythony run --mode binary` per the leaf gate text and the 02.05 conformance
# note, and every piece is overridable: APX_IMPL (impl command prefix),
# APX_MODE_TIER0_CMD / APX_MODE_ADAPTIVE_CMD / APX_MODE_BINARY_CMD (full per-mode
# command lines, script path appended), APX_T08_TIMEOUT (per-run seconds),
# APX_T08_REPORT (benchmark JSON path). Gap G-1.

import json
import os
import re
import shlex
import shutil
import statistics
import subprocess
import sys
import time
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
CORPUS_ROOT = REPO_ROOT / "tests" / "corpus"
PROBE_DIR = CORPUS_ROOT / "tier1-transparency"
BENCH_DIR = PROBE_DIR / "bench"
# Reference discovery: unified env+PATH ladder (robustness-ci 01.01); see
# tests/tools/_ref_python.py for the AP_REQUIRE_REFERENCE=1 skip->fail policy.
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
from _ref_python import reference_or_skip  # noqa: E402

SPECS = {
    "S-01": REPO_ROOT / "docs" / "spec" / "01-values-and-objects.md",
    "S-02": REPO_ROOT / "docs" / "spec" / "02-control-flow.md",
    "S-03": REPO_ROOT / "docs" / "spec" / "03-classes.md",
}
CONTRACT = REPO_ROOT / "docs" / "compat-contract.md"

XFAIL_REASON = "awaits 02.05 tier-1 (CLIF/IC/OSR/deopt) + antipythony run mode flags"

RUN_TIMEOUT = int(os.environ.get("APX_T08_TIMEOUT", "120"))

PROBES = sorted(p for p in PROBE_DIR.glob("*.py"))
BENCHES = sorted(p for p in BENCH_DIR.glob("*.py"))
SLICE = PROBES + BENCHES  # everything authored by T-08
# Whole-corpus glob AT RUN TIME (C-33: corpus only grows; the gate always covers
# whatever exists at gate time — values, classes, exceptions, tier1 slices, ...).
ALL_CORPUS = sorted(CORPUS_ROOT.rglob("*.py"))


def _ids(paths):
    return [str(p.relative_to(REPO_ROOT)) for p in paths]


xfail_ap = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)


def _scrubbed_env():
    return {k: os.environ[k] for k in ("PATH", "HOME", "LANG") if k in os.environ}


def _run_ref(script: Path) -> subprocess.CompletedProcess:
    return subprocess.run([reference_or_skip(), str(script)], capture_output=True,
                          env=_scrubbed_env(), cwd=REPO_ROOT, timeout=60)


def _expected_files(script: Path):
    exp = script.parent / "expected"
    stem = script.stem
    return exp / (stem + ".stdout"), exp / (stem + ".stderr"), exp / (stem + ".exit")


def _antipythony_prefix() -> list[str]:
    override = os.environ.get("APX_IMPL")
    if override:
        return shlex.split(override)
    found = shutil.which("antipythony")
    if found:
        return [found, "run"]
    for profile in ("debug", "release"):
        cand = REPO_ROOT / "target" / profile / "antipythony"
        if cand.is_file():
            return [str(cand), "run"]
    raise FileNotFoundError(
        "no `antipythony` impl command found (set APX_IMPL=<cmdline> to override) "
        "— 02.05 not landed"
    )


MODES = ("tier0", "adaptive", "binary")


def mode_cmd(mode: str) -> list[str]:
    """Command line for one tier/mode; the script path is appended by callers.
    Flag vocabulary is gap G-1 (docs do not pin it); full override per mode."""
    env_key = {"tier0": "APX_MODE_TIER0_CMD",
               "adaptive": "APX_MODE_ADAPTIVE_CMD",
               "binary": "APX_MODE_BINARY_CMD"}[mode]
    override = os.environ.get(env_key)
    if override:
        return shlex.split(override)
    prefix = _antipythony_prefix()  # raises FileNotFoundError until the impl exists
    if mode == "tier0":
        return prefix + ["--tier", "0"]
    if mode == "adaptive":
        return prefix + ["--tier", "adaptive"]
    if mode == "binary":
        return prefix + ["--mode", "binary"]
    raise ValueError(mode)


def _run_mode(mode: str, script: Path) -> subprocess.CompletedProcess:
    return subprocess.run(mode_cmd(mode) + [str(script)], capture_output=True,
                          env=_scrubbed_env(), cwd=REPO_ROOT, timeout=RUN_TIMEOUT)


def _header(script: Path) -> str:
    return script.read_text(encoding="utf-8").splitlines()[0]


# ---------------------------------------------------------------------------
# (A) Fixture re-verification — GREEN today. These pin the semantics reference
#     bytes that every mode must reproduce, and police the slice's own rules.
# ---------------------------------------------------------------------------


def test_probe_slice_quota():
    # Task contract: >= 12 specialization probes. Floor only; corpus grows (C-33).
    assert len(PROBES) >= 12, f"only {len(PROBES)} probes in {PROBE_DIR}"
    assert len(BENCHES) >= 4, "speed slice needs arithmetic + string programs"
    assert len(ALL_CORPUS) >= 100, (
        "whole-corpus transparency gate presumes an existing tree; if this fails, "
        "the corpus was shrunk — C-33 violation, not a T-08 bug"
    )


@pytest.mark.parametrize("script", SLICE, ids=_ids(SLICE))
def test_expected_triple_exists(script):
    for f in _expected_files(script):
        assert f.is_file(), f"missing {f.relative_to(REPO_ROOT)}"


@pytest.mark.parametrize("script", SLICE, ids=_ids(SLICE))
def test_header_cites_existing_clauses(script):
    # First line: '# Cites: S-01-nn, ... , C-31 (tier modes invisible)' style.
    first = _header(script)
    assert first.startswith("# Cites:"), f"{script.name}: missing '# Cites:' header"
    cited = re.findall(r"\b(S-\d\d-\d+|C-\d+)\b", first)
    assert cited, f"{script.name}: no clause ids in header"
    # Every C-id cited must exist as a live clause in the contract.
    contract_text = CONTRACT.read_text()
    for cid in {c for c in cited if c.startswith("C-")}:
        assert f"### {cid} " in contract_text, (
            f"{script.name}: cites {cid}, not a clause in docs/compat-contract.md"
        )
    assert "C-31" in cited, f"{script.name}: every tier-transparency case cites C-31"
    for fam, doc in SPECS.items():
        ids = {s for s in cited if s.startswith(fam)}
        if not ids:
            continue
        spec_ids = set(re.findall(r"\*\*(" + fam.replace("-", "-") + r"-\d+)\b",
                                  doc.read_text()))
        unknown = ids - spec_ids
        assert not unknown, f"{script.name}: cites ids absent from {doc.name}: {sorted(unknown)}"


@pytest.mark.parametrize("script", SLICE, ids=_ids(SLICE))
def test_stresses_header_present(script):
    # Each program documents the tier-1 mechanism it forces (probe table in
    # plans/antipythony/tests/T-08-tier1-transparency.md must match the files).
    lines = script.read_text(encoding="utf-8").splitlines()[:6]
    assert any(l.startswith("# Stresses:") for l in lines), (
        f"{script.name}: missing '# Stresses:' annotation"
    )


@pytest.mark.parametrize("script", SLICE, ids=_ids(SLICE))
def test_reference_reproduces_expected_bytes(script):
    # The pinned triples are the semantics reference (C-01). Drift here is a
    # reference-build finding (GAP G-7: recorded on 3.14.7, C-01 pins 3.14.0),
    # NOT a gate finding.
    proc = _run_ref(script)
    out_f, err_f, rc_f = _expected_files(script)
    assert proc.stdout == out_f.read_bytes(), (
        f"{script.name}: stdout drift\n got={proc.stdout!r}\nwant={out_f.read_bytes()!r}"
    )
    assert _mask_root(proc.stderr) == _mask_root(err_f.read_bytes()), (
        f"{script.name}: stderr drift: {proc.stderr!r}"
    )
    assert str(proc.returncode) == rc_f.read_text().strip(), (
        f"{script.name}: rc {proc.returncode} != {rc_f.read_text().strip()}"
    )


@pytest.mark.parametrize("script", SLICE, ids=_ids(SLICE))
def test_expected_streams_are_clean(script):
    # All probes swallow any expected errors in-script: rc 0, empty stderr,
    # newline-terminated stdout. Keeps traceback path text out of expected
    # bytes (spec 02 convention) and pins C-30-style clean exit.
    out_f, err_f, rc_f = _expected_files(script)
    assert rc_f.read_text().strip() == "0", script.name
    assert err_f.read_bytes() == b"", script.name
    assert out_f.read_bytes().endswith(b"\n"), script.name


@pytest.mark.parametrize("script", SLICE, ids=_ids(SLICE))
def test_no_nondeterministic_addresses_leak(script):
    out_f, err_f, _rc = _expected_files(script)
    blob = out_f.read_bytes() + err_f.read_bytes()
    m = re.search(rb"0x[0-9a-fA-F]{4,}", blob)
    assert m is None, f"{script.name}: address-like token {m.group()!r} in expected bytes (C-24)"


@pytest.mark.parametrize("script", SLICE, ids=_ids(SLICE))
def test_no_timing_or_identity_apis(script):
    # C-24: timing/id are non-conformance channels; a transparency probe that
    # reads a clock could flap between modes. D3/C-19: refcount APIs prohibited.
    banned = ("getrefcount", "gettotalrefcount", "id(", "time.", "perf_counter",
              "import time", "import random", "random.", "gc.is_tracked", "clock()")
    offenders = []
    for i, line in enumerate(script.read_text(encoding="utf-8").splitlines(), 1):
        if line.strip().startswith("#"):
            continue
        for b in banned:
            if b in line:
                offenders.append(f"{script.name}:{i}: {line.strip()}")
    assert not offenders, "\n".join(offenders)


@pytest.mark.parametrize("script", SLICE, ids=_ids(SLICE))
def test_iteration_budget_bound(script):
    # Task rule: hot loops in the 10k–100k band (tier-0 runs them SLOW by design;
    # millions of iterations would make the conformance run impractical). Every
    # numeric literal passed to range() must be <= 200_000.
    text = script.read_text(encoding="utf-8")
    bad = []
    for m in re.finditer(r"range\(([^)]*)\)", text):
        args = [a.strip() for a in m.group(1).split(",") if a.strip().isdigit()]
        for a in args:
            if int(a) > 200_000:
                bad.append(f"range({m.group(1)})")
    assert not bad, f"{script.name}: oversized loop bounds {bad} (budget cap, G-5)"


def test_probe_table_sync():
    # The mechanism table in the T-08 doc lists exactly the probe files present.
    doc = REPO_ROOT / "plans" / "antipythony" / "tests" / "T-08-tier1-transparency.md"
    text = doc.read_text()
    for p in PROBES:
        assert p.name in text, f"{p.name} missing from probe table in {doc.name}"
    # Mechanism coverage floors (C-31 Verify: OSR/deopt must be exercised):
    joined = "\n".join(p.read_text() for p in PROBES).lower()
    for mechanism in ("osr", "deopt", "inline", "generator", "megamorphic", "slots"):
        assert mechanism in joined, f"no probe exercises {mechanism!r} machinery"


# ---------------------------------------------------------------------------
# (B1) Cross-mode byte-identity — whole-corpus glob vs tier0/adaptive/binary.
#      XFAIL until 02.05 (+ mode flags). C-31: "Forced tier-0, adaptive, and AoT
#      binary modes MUST agree on every corpus case." C-29/C-28: the exit-code
#      component of that agreement. C-24/C-27: byte-exact, separate streams,
#      no normalization.
# ---------------------------------------------------------------------------


@xfail_ap
@pytest.mark.parametrize("script", ALL_CORPUS, ids=_ids(ALL_CORPUS))
def test_cross_mode_byte_identity(script):
    runs = {mode: _run_mode(mode, script) for mode in MODES}
    base = runs["tier0"]
    for mode in ("adaptive", "binary"):
        got = runs[mode]
        assert got.stdout == base.stdout, (
            f"{script.name}: stdout differs under {mode} vs tier-0 "
            f"(C-31 tier-1 invisible except by timing)"
        )
        assert got.stderr == base.stderr, f"{script.name}: stderr differs under {mode} (C-31)"
        assert got.returncode == base.returncode, (
            f"{script.name}: exit code {got.returncode} under {mode} != "
            f"{base.returncode} under tier-0 (C-31 + C-29/C-28)"
        )
    # When the tree carries pinned reference triples, all modes must also equal
    # the CPython reference bytes (values/tier1 use .stdout/.stderr/.exit;
    # classes/exceptions use .out/.err/.rc).
    exp = script.parent / "expected"
    for suffixes in ((".stdout", ".stderr", ".exit"), (".out", ".err", ".rc")):
        trio = [exp / (script.stem + s) for s in suffixes]
        if all(t.is_file() for t in trio):
            assert base.stdout == trio[0].read_bytes(), f"{script.name}: tier-0 != reference stdout"
            assert base.stderr == trio[1].read_bytes(), f"{script.name}: tier-0 != reference stderr"
            assert str(base.returncode) == trio[2].read_text().strip(), (
                f"{script.name}: tier-0 != reference rc"
            )
            break


@xfail_ap
def test_cross_mode_suite_runs_over_grown_corpus():
    # Sanity wrapper proving the glob is live at gate time (C-33): the suite
    # parametrization must include every directory that exists under tests/corpus.
    dirs = {p.parent.name for p in ALL_CORPUS if p.parent != CORPUS_ROOT}
    assert {"values", "classes", "exceptions", "tier1-transparency"} <= dirs, (
        f"corpus tree unexpectedly missing slices: {sorted(dirs)}"
    )
    proc = _run_mode("adaptive", ALL_CORPUS[0])
    assert proc.returncode == 0


# ---------------------------------------------------------------------------
# (B2) Specialization-probe differential: each probe, each mode, vs pinned
#      reference bytes. A deopt that restores a WRONG tier-0 mid-state corrupts
#      loop-carried/exception/generator state — visible as output drift.
# ---------------------------------------------------------------------------


@xfail_ap
@pytest.mark.parametrize("mode", MODES)
@pytest.mark.parametrize("script", PROBES, ids=_ids(PROBES))
def test_probe_matches_reference_bytes(script, mode):
    proc = _run_mode(mode, script)
    out_f, err_f, rc_f = _expected_files(script)
    assert proc.stdout == out_f.read_bytes(), (
        f"{script.name} under {mode}: stdout differs from the pinned python3.14 "
        f"bytes — specialization/deopt corrupted observable state (C-31)\n"
        f" got={proc.stdout!r}\nwant={out_f.read_bytes()!r}"
    )
    assert proc.stderr == err_f.read_bytes(), f"{script.name} under {mode}: stderr drift (C-31/C-25)"
    assert proc.returncode == int(rc_f.read_text()), f"{script.name} under {mode}: rc differs (C-31/C-29)"


@xfail_ap
def test_osr_side_effects_exactly_once_adaptive():
    # C-31 Verify verbatim: side effects across an OSR/deopt boundary run
    # exactly once (no duplicated or dropped prints/accumulations). t1_004 is
    # built so a duplicated append or dropped accumulation changes len(marks),
    # acc, and the sampled marks — all pinned against the reference above; this
    # is the explicit adaptive-mode spot.
    proc = _run_mode("adaptive", PROBE_DIR / "t1_004_osr_side_effects_once.py")
    expected = (PROBE_DIR / "expected" / "t1_004_osr_side_effects_once.stdout").read_bytes()
    assert proc.stdout == expected
    assert proc.returncode == 0


# ---------------------------------------------------------------------------
# (B3) The 02.05 conformance gate, verbatim half: "adaptive-mode full corpus
#      green" — expressed suite-side as every corpus case matching its pinned
#      reference triple under adaptive mode. (Per-case byte identity across all
#      three modes is B1/B2; ap-conform remains the machine-readable gate at
#      02.06/T-09 level.)
# ---------------------------------------------------------------------------


@xfail_ap
def test_adaptive_mode_full_corpus_green():
    failures = []
    for script in ALL_CORPUS:
        exp = script.parent / "expected"
        proc = _run_mode("adaptive", script)
        ref = _run_ref(script)
        if (proc.stdout, proc.stderr, proc.returncode) != (ref.stdout, ref.stderr, ref.returncode):
            failures.append(script.relative_to(REPO_ROOT).as_posix())
    assert not failures, "adaptive-mode corpus failures:\n" + "\n".join(failures)


# ---------------------------------------------------------------------------
# (B4) Speed-gate STUB — measured, not asserted-by-faith. Gate text (02.05):
#      ">=1.5x geomean over tier-0 on the arithmetic/string benchmark slice."
#      Timing is NOT a conformance channel (C-24), so this never compares
#      against python3.14; it compares the impl's own adaptive vs forced tier-0.
#      Conservative estimator per program: min(tier-0 times) / max(adaptive
#      times) — punishes noise in tier-0's favour being counted as speedup…
#      (min of tier-0 is the fastest honest tier-0; max of adaptive is the
#      slowest honest adaptive; their ratio is a lower bound on speedup).
#      See T-08 doc GAP G-3 for the floor-tolerance decision.
# ---------------------------------------------------------------------------

SPEED_FLOOR = 1.5
REPEATS = int(os.environ.get("APX_T08_SPEED_REPEATS", "3"))
WARMUP = 1


def _timed_runs(mode: str, script: Path) -> list[float]:
    for _ in range(WARMUP):
        _run_mode(mode, script)
    times = []
    for _ in range(REPEATS):
        t0 = time.perf_counter()
        proc = _run_mode(mode, script)
        dt = time.perf_counter() - t0
        assert proc.returncode == 0, f"{script.name} under {mode} failed before timing"
        times.append(dt)
    return times


def _speed_report_path() -> Path:
    override = os.environ.get("APX_T08_REPORT")
    if override:
        return Path(override)
    return Path(__file__).resolve().parent / "bench_report.json"


@xfail_ap
def test_speed_gate_geomean_over_tier0():
    per_program = {}
    ratios = []
    for script in BENCHES:
        t0_times = _timed_runs("tier0", script)
        ad_times = _timed_runs("adaptive", script)
        conservative = min(t0_times) / max(ad_times)
        per_program[script.name] = {
            "tier0_times_s": [round(v, 4) for v in t0_times],
            "adaptive_times_s": [round(v, 4) for v in ad_times],
            "speedup_conservative": round(conservative, 4),
        }
        ratios.append(conservative)
    geomean = statistics.geometric_mean(ratios)
    report = {
        "gate": "02.05: >=1.5x geomean over tier-0 on arithmetic/string benchmark slice",
        "bench_slice": [p.name for p in BENCHES],
        "repeats": REPEATS,
        "warmup": WARMUP,
        "floor": SPEED_FLOOR,
        "geomean_speedup_conservative": round(geomean, 4),
        "per_program": per_program,
    }
    rp = _speed_report_path()
    rp.parent.mkdir(parents=True, exist_ok=True)
    rp.write_text(json.dumps(report, indent=2))
    assert geomean >= SPEED_FLOOR, (
        f"adaptive geomean speedup {geomean:.3f}x < {SPEED_FLOOR}x gate "
        f"(report: {rp}); timings are machine-dependent — see T-08 G-3"
    )
