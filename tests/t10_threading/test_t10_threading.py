# T-10 — threading (gates implementation leaf 02.07 Native threading).
#
# Authorship: written from the spec pack ONLY (docs/spec/02-control-flow.md,
# docs/spec/05-builtins-stdlib-v1.md — the threading row of §5.3.2,
# docs/compat-contract.md C-14/C-19/C-28/D1, plans/antipythany/master.md,
# plans/antipythany/tests/README.md, and leaf 02.07's Expose/Gate in
# plans/antipythany/02-native-backend/orchestrator.md). The author has not read
# any implementation source under crates/.
#
# This file has THREE kinds of tests:
#
#  (A) FIXTURE self-checks (must be GREEN today; they test the corpus, not
#      antipythany): every tests/corpus/threading/T1*.py has an expected
#      .stdout/.stderr/.exit triple produced by real /opt/homebrew/bin/python3.14
#      runs; each case is run THREE times here and all three runs must be
#      byte-identical on all channels (determinism is the enemy — a 3-run drift
#      means the case has an unmasked race and must be REWRITTEN into
#      rendezvous form, never deleted); header-citation lint; no-sleep lint
#      (sleep-based timing is banned for this slice — spec 05 threading row:
#      "corpus MAY assert logical ordering only, never wall-clock timing");
#      refcount-API lint (C-19/D3); and the reference-vs-reference differential
#      through `ap-conform run --impl <python3.14>` over the whole slice.
#
#  (B) ANTIPYTHANY-side gate tests (xfail(strict=False) until leaf 02.07
#      lands): `ap-conform run --corpus tests/corpus/threading/*.py --impl
#      <antipythany>` must go green on the whole slice (leaf Gate part 1), plus
#      per-channel spot gates against pinned triples, plus the GIL-free
#      correctness differential (T120: parallel CPU results == sequential
#      baseline bytes).
#
#  (C) SCALING harness (leaf Gate part 2: "scaling case >= 3x on 4 cores" —
#      MEASURED, never asserted against the reference). One green plumbing test
#      runs T121 through serial/parallel modes on the reference and only RECORDS
#      timings (CPython with GIL is expected near 1.0x — asserting on that would
#      be asserting CPython's own GIL away). The >=3x gate itself is an xfail
#      test over the antipythany impl, guarded by a core-count skip (>= 4
#      logical cores; this authoring host reports 24).
#
# See plans/antipythany/tests/T-10-threading.md for the case table, per-case
# determinism rules, and CONTRACT-GAPS.

import json
import os
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS_DIR = REPO_ROOT / "tests" / "corpus" / "threading"
EXPECTED_DIR = CORPUS_DIR / "expected"
REF_PY = "/opt/homebrew/bin/python3.14"
AP_CONFORM = os.environ.get(
    "AP_CONFORM", str(REPO_ROOT / "target" / "debug" / "ap-conform")
)
APX_IMPL = os.environ.get("APX_IMPL", "")  # e.g. "target/debug/antipythany run"

XFAIL_REASON = "awaits 02.07 native threading (antipythany impl side)"

CORES = os.cpu_count() or 1


def cases():
    return sorted(CORPUS_DIR.glob("T1*.py"))


ALL_CASES = cases()


def case_ids():
    return [p.stem for p in ALL_CASES]


def run_ref(case, env_extra=None, timeout=180):
    env = dict(os.environ)
    if env_extra:
        env.update(env_extra)
    return subprocess.run(
        [REF_PY, str(case)], capture_output=True, timeout=timeout,
        cwd=str(REPO_ROOT), env=env,
    )


# ---------------------------------------------------------------------------
# (A) Fixture self-checks — green today.
# ---------------------------------------------------------------------------


def test_corpus_slice_is_populated():
    # Leaf 02.07 Gate: "corpus threading slice green". The slice must exist
    # with >= 14 cases covering the threading v1 subset of spec 05 §5.3.2.
    assert len(ALL_CASES) >= 14, f"only {len(ALL_CASES)} cases in {CORPUS_DIR}"


@pytest.mark.parametrize("case", ALL_CASES, ids=case_ids())
def test_expected_triple_exists(case):
    stem = case.stem
    for suffix in (".stdout", ".stderr", ".exit"):
        assert (EXPECTED_DIR / (stem + suffix)).is_file(), (
            f"missing expected/{stem}{suffix}"
        )


@pytest.mark.parametrize("case", ALL_CASES, ids=case_ids())
def test_expected_stdout_nonempty_and_exit_pinned(case):
    out = (EXPECTED_DIR / (case.stem + ".stdout")).read_bytes()
    assert out != b"", f"{case.stem}: empty expected stdout"
    rc = (EXPECTED_DIR / (case.stem + ".exit")).read_bytes()
    assert rc.strip().lstrip(b"-").isdigit(), (
        f"{case.stem}: bad .exit content {rc!r}"
    )


@pytest.mark.parametrize("case", ALL_CASES, ids=case_ids())
def test_expected_bytes_stable(case):
    # Re-run the reference and byte-compare all three channels against the
    # pinned triple (C-24/C-27: stdout, stderr, exit code; no trimming, no
    # normalization). Drift here means a race leaked into output: the case is
    # FIXED BY REWRITING the rendezvous, never weakened (corpus rule).
    proc = run_ref(case)
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
def test_case_is_three_run_deterministic(case):
    # THE core rule of this slice: every case must produce byte-identical
    # stdout+stderr+exit across THREE consecutive reference runs. A threading
    # case that flakes here is nondeterministic BY CONSTRUCTION and cannot pin
    # anything; per the corpus-grows rule it gets rewritten into rendezvous
    # form (Event/Barrier/queue/join), not deleted.
    results = []
    for _ in range(3):
        proc = run_ref(case)
        results.append((proc.stdout, proc.stderr, proc.returncode))
    assert results[0] == results[1] == results[2], (
        f"{case.stem}: 3-run byte drift:\n"
        + "\n".join(repr(r)[:200] for r in results)
    )


@pytest.mark.parametrize("case", ALL_CASES, ids=case_ids())
def test_case_header_cites_clauses(case):
    # tests/README.md corpus rule: every case cites spec clause ids.
    text = case.read_text(encoding="utf-8")[:900]
    assert "# Cites:" in text and ("S-05-" in text or "C-" in text or "S-02-" in text), (
        f"{case.stem}: header does not cite clause ids"
    )


def test_no_sleep_based_timing_in_corpus():
    # Spec 05 threading row: "corpus MAY assert logical ordering only (joined
    # programs), never wall-clock timing". Sleep-based ordering is banned for
    # this slice: rendezvous primitives (Event/Barrier/Condition/queue/join)
    # are the only permitted synchronization. (time.sleep must not even appear;
    # bounded wait(timeout=...) appears only in T113 where the RESULT is
    # deterministic — barrier.broken — not an ordering.)
    offenders = []
    for case in ALL_CASES:
        for i, line in enumerate(case.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("#"):
                continue
            if "sleep(" in line:
                offenders.append(f"{case.stem}:{i}: {line.strip()}")
    assert not offenders, "sleep-based timing in corpus:\n" + "\n".join(offenders)


def test_no_forbidden_refcount_apis():
    # C-19/D3: refcount-introspection APIs are corpus-prohibited; the harness
    # rejects such cases. C-14-adjacent: gc import is prohibited (spec 05
    # S-05-21). id() results are never printed either (S-06-17 shape).
    banned = ("getrefcount", "gettotalrefcount", "getallocatedblocks",
              "import gc")
    offenders = []
    for case in ALL_CASES:
        for i, line in enumerate(case.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("#"):
                continue
            for b in banned:
                if b in line:
                    offenders.append(f"{case.stem}:{i}: {line.strip()}")
    assert not offenders, "\n".join(offenders)


def test_out_of_v1_modules_not_used():
    # S-05-19: concurrent.futures is OUT-of-v1, so this slice uses raw
    # Thread/queue/queue.Queue rendezvous only. If futures ever appear here it
    # is a spec-05 PR first. Also random/multiprocessing are OUT.
    banned = ("concurrent.futures", "ThreadPoolExecutor", "multiprocessing",
              "import random", "subprocess")
    offenders = []
    for case in ALL_CASES:
        for i, line in enumerate(case.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("#"):
                continue
            for b in banned:
                if b in line:
                    offenders.append(f"{case.stem}:{i}: {line.strip()}")
    assert not offenders, "\n".join(offenders)


def test_scaling_case_mode_guard_result_identity():
    # The scaling case T121 must print IDENTICAL stdout bytes in default,
    # serial and parallel modes — the pinned conformance channel carries no
    # timing (D4: no perf assertions in conformance cases). Timings go to
    # stderr only under explicit T10_MODE, and are never in the triple.
    case = CORPUS_DIR / "T121-scaling-parallel-cpu.py"
    base = run_ref(case).stdout
    for mode in ("serial", "parallel"):
        proc = run_ref(case, env_extra={"T10_MODE": mode})
        assert proc.returncode == 0, (mode, proc.stderr)
        assert proc.stdout == base, f"T121 stdout differs in {mode} mode"
        assert b"perf" not in proc.stdout


@pytest.mark.skipif(not Path(AP_CONFORM).is_file(), reason="ap-conform bin missing")
def test_apconform_self_comparison_green():
    # 00.04 harness contract re-proven on THIS slice: reference vs reference
    # through ap-conform must pass byte-exactly on all three channels,
    # including the exception-in-thread stderr report of T109.
    proc = subprocess.run(
        [AP_CONFORM, "run", "--corpus", "tests/corpus/threading/*.py",
         "--impl", REF_PY, "--timeout", "60", "--json"],
        capture_output=True, text=True, timeout=600, cwd=str(REPO_ROOT),
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr
    data = json.loads(proc.stdout)
    summary = data["summary"]
    assert summary["failed"] == 0 and summary["errors"] == 0 and summary["timeouts"] == 0, (
        f"harness self-comparison not green: {summary}"
    )
    assert summary["passed"] == summary["total"] == len(ALL_CASES), summary


# ---------------------------------------------------------------------------
# (B) Antipythany-side gates — xfail(strict=False) until 02.07 lands.
# ---------------------------------------------------------------------------

xfail_ap = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)


def _ap_conform_available():
    return Path(AP_CONFORM).is_file()


def _impl_cmd():
    if APX_IMPL:
        return shlex.split(APX_IMPL)
    runner = REPO_ROOT / "target" / "debug" / "antipythany"
    if runner.is_file():
        return [str(runner), "run"]
    return None


@pytest.mark.skipif(not _ap_conform_available(), reason="ap-conform bin missing")
@xfail_ap
def test_apconform_antipythany_full_slice():
    # Leaf 02.07 Gate part 1: "corpus threading slice green".
    impl = _impl_cmd()
    assert impl is not None, "no antipythany runner; set APX_IMPL"
    proc = subprocess.run(
        [AP_CONFORM, "run", "--corpus", "tests/corpus/threading/*.py",
         "--impl", *impl, "--timeout", "60"],
        capture_output=True, text=True, timeout=1800, cwd=str(REPO_ROOT),
    )
    assert proc.returncode == 0, proc.stdout[-4000:] + proc.stderr[-2000:]


@pytest.mark.skipif(not _ap_conform_available(), reason="ap-conform bin missing")
@xfail_ap
@pytest.mark.parametrize(
    "stem",
    ["T101-start-join-completion-order", "T103-queue-producer-consumer",
     "T107-daemon-exits-with-main", "T108-thread-naming-repr",
     "T109-exception-in-thread", "T113-barrier",
     "T114-system-exit-in-thread", "T122-no-refcount-visibility"],
)
def test_apconform_spot_cases_against_pinned_bytes(stem):
    # Per-channel gate (C-24/C-27): run ONE case under the impl and diff the
    # three channels against the pinned triples. Spot set: start/join
    # aggregation, queue rendezvous, daemon-exits-with-main (C-28-adjacent),
    # naming/active_count/repr heads, thread excepthook stderr report (S-02-35
    # shape inside threads), Barrier + BrokenBarrierError, SystemExit-in-thread
    # (C-28), no-refcount-visibility (C-19/D1/D3).
    impl = _impl_cmd()
    assert impl is not None, "no antipythany runner; set APX_IMPL"
    case = CORPUS_DIR / (stem + ".py")
    proc = subprocess.run([*impl, str(case)], capture_output=True, timeout=120,
                          cwd=str(REPO_ROOT))
    assert proc.stdout == (EXPECTED_DIR / (stem + ".stdout")).read_bytes(), stem
    assert proc.stderr == (EXPECTED_DIR / (stem + ".stderr")).read_bytes(), stem
    assert proc.returncode == int(
        (EXPECTED_DIR / (stem + ".exit")).read_bytes()
    ), stem
    # NOTE: T109's pinned stderr contains the reference's own threading.py
    # bootstrap frames with absolute lib paths — see CONTRACT-GAPS G-1 in
    # plans/antipythany/tests/T-10-threading.md. This spot gate stays xfail
    # until that gap is resolved by spec/impl, not by re-pinning.


@xfail_ap
def test_gil_free_correctness_differential():
    # Leaf 02.07 Expose: "no GIL". CORRECTNESS probe T120: 4 threads of
    # independent CPU work must produce byte-identical stdout to the pinned
    # reference triple (sequential == parallel). SPEED is not asserted here
    # (D4); the scaling harness below measures it.
    impl = _impl_cmd()
    assert impl is not None, "no antipythany runner; set APX_IMPL"
    case = CORPUS_DIR / "T120-gil-free-cpu-probe.py"
    proc = subprocess.run([*impl, str(case)], capture_output=True, timeout=300,
                          cwd=str(REPO_ROOT))
    stem = "T120-gil-free-cpu-probe"
    assert proc.stdout == (EXPECTED_DIR / (stem + ".stdout")).read_bytes(), stem
    assert proc.stderr == (EXPECTED_DIR / (stem + ".stderr")).read_bytes(), stem
    assert proc.returncode == int(
        (EXPECTED_DIR / (stem + ".exit")).read_bytes()
    ), stem


# ---------------------------------------------------------------------------
# (C) Scaling harness — measured-not-asserted for the reference; the >=3x
#     gate over antipythany is xfail until 02.07 lands.
# ---------------------------------------------------------------------------


def _measure(mode, cmd_runner):
    import re
    import time as _time

    env = dict(os.environ)
    env["T10_MODE"] = mode
    case = CORPUS_DIR / "T121-scaling-parallel-cpu.py"
    best = float("inf")
    for _ in range(3):
        t0 = _time.perf_counter()
        proc = subprocess.run([*cmd_runner, str(case)], capture_output=True,
                              timeout=300, cwd=str(REPO_ROOT), env=env)
        wall = _time.perf_counter() - t0
        assert proc.returncode == 0, (mode, proc.stderr[-500:])
        best = min(best, wall)
    assert best != float("inf")
    return best


def test_scaling_harness_plumbing_records_reference_timings():
    # GREEN plumbing check (tests the harness, not antipythany): T121 runs in
    # both modes under the reference and the harness records finite positive
    # timings. We PRINT the speedup and assert nothing about its magnitude —
    # CPython 3.14 here is GIL-enabled (sys._is_gil_enabled() is True), so its
    # speedup is ~1.0x and pinning that would contradict "measured-not-
    # asserted". The >=3x gate is only over the GIL-free antipythany impl.
    serial = _measure("serial", [REF_PY])
    parallel = _measure("parallel", [REF_PY])
    assert serial > 0 and parallel > 0
    speedup = serial / parallel
    print(f"\nT-10 scaling harness: cores={CORES} serial={serial:.3f}s "
          f"parallel={parallel:.3f}s speedup={speedup:.2f}x (reference, "
          f"GIL-enabled; recorded only)")


@pytest.mark.skipif(CORES < 4, reason=f"scaling gate needs >=4 cores (host has {CORES})")
@xfail_ap
def test_scaling_gate_3x_on_4cores_antipythany():
    # Leaf 02.07 Gate part 2: "scaling case >= 3x on 4 cores". Run T121 in
    # serial and parallel modes under the antipythany impl, best-of-3 wall
    # times, and assert speedup >= 3.0. Guarded by machine core count: this
    # authoring host has 24 logical cores; CI boxes with <4 cores SKIP
    # (annotated above) rather than false-fail.
    impl = _impl_cmd()
    assert impl is not None, "no antipythany runner; set APX_IMPL"
    serial = _measure("serial", impl)
    parallel = _measure("parallel", impl)
    speedup = serial / parallel
    print(f"T-10 scaling gate: cores={CORES} speedup={speedup:.2f}x")
    assert speedup >= 3.0, (
        f"GIL-free scaling gate: expected >=3.0x on >=4 cores, got {speedup:.2f}x "
        f"(serial {serial:.3f}s / parallel {parallel:.3f}s)"
    )
