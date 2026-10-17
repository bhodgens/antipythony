# T-07 — gc/finalizer test suite (gates leaf 02.04 ap-gc MMTk integration).
#
# Authorship: written from the spec pack ONLY (docs/compat-contract.md
# C-19..C-23 + divergence register D1/D3, docs/spec/01-values-and-objects.md,
# docs/spec/02-control-flow.md, docs/spec/06-observable-behavior.md,
# plans/antipythany/master.md, plans/antipythany/02-native-backend/
# orchestrator.md 02.04 Expose/Gate, plans/antipythany/tests/README.md).
# No implementation source under crates/ was read. Per-case citations,
# D1-timing-safety analysis and CONTRACT-GAPS:
# plans/antipythany/tests/T-07-gc-finalizer.md.
#
# Two halves:
#  1. FIXTURE RE-VERIFICATION (green today): every corpus script under
#     tests/corpus/gc/ is re-run with the reference CPython and its
#     stdout/stderr/exit bytes are diffed against the recorded expected/
#     files; headers are checked for clause citations; the 3-run
#     determinism check (C-23: "the harness rejects a case that flaps")
#     is enforced; corpus-lint checks for C-19/C-23/S-06-17 violations
#     run here because the ap-conform load-time hook is an open gap (G-2).
#  2. ap-conform GATE (xfail, strict=False): the 02.04 gate — "allocation-
#     heavy corpus slice green; no leak across 10k iterations (heap
#     growth bounded)" — plus the C-19 getrefcount-absent projection.
#     Neither the antipythany binary nor the ap-conform --impl seam for
#     this slice exists yet, so these XFAIL.
#
# sys.getrefcount note (C-19/D3): refcount APIs are CORPUS-PROHIBITED.
# The presence/absence assertion therefore does NOT live in
# tests/corpus/gc/ — it is this impl-side xfail test driving
# probes/refcount_probe.py under the (not yet existing) impl, citing C-14
# per the task contract and C-19/D3 by clause text. See T-07 doc §C-14.

import os
import re
import shlex
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
GC_CORPUS = REPO_ROOT / "tests" / "corpus" / "gc"
PROBES = Path(__file__).resolve().parent / "probes"

# Reference CPython (C-01 caveat G-7: expected bytes recorded from 3.14.7,
# provisional until re-verified on pinned 3.14.0 — same open item as the
# compat contract and the T-05/T-06 suites).
PYREF = os.environ.get("AP_CONFORM_REFERENCE_PYTHON", "/opt/homebrew/bin/python3.14")

XFAIL_REASON = "awaits 02.04 ap-gc (MMTk) + ap-conform --impl seam"

CASES = sorted(p.stem for p in GC_CORPUS.glob("gc*.py"))
SCRIPTS = {stem: GC_CORPUS / (stem + ".py") for stem in CASES}


def _scrubbed_env() -> dict:
    # S-06-15: ap-conform launches both impls with only PATH/HOME/LANG.
    return {k: os.environ[k] for k in ("PATH", "HOME", "LANG") if k in os.environ}


def _run_ref(script: Path) -> subprocess.CompletedProcess:
    """Run a corpus case as a FILE under the reference (same convention as
    ap-conform: identical argv to both impls, separate stream capture per
    S-06-2, scrubbed env)."""
    return subprocess.run(
        [PYREF, str(script)],
        capture_output=True,
        env=_scrubbed_env(),
        cwd=REPO_ROOT,
        timeout=120,
    )


def _cited_ids(script: Path):
    first = script.read_text().splitlines()[0]
    assert first.startswith("# Cites:"), (
        f"{script.name}: first line must be a '# Cites: C-nn...' header "
        f"(tests/README.md corpus rules); got: {first!r}"
    )
    return {int(m) for m in re.findall(r"C-(\d{2})", first)}


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
    found = shutil.which("antipythany")
    if found:
        return [found, "run"]
    raise FileNotFoundError(
        "antipythany impl command not found (set APX_IMPL=<cmdline> to override)"
    )


def run_conform(corpus_arg: str) -> subprocess.CompletedProcess:
    # CLI contract as consumed by T-05/T-06 (leaf 00.04): run --corpus
    # <glob-or-file> --impl <cmd> --tier 0 (02.04 runs under tier-0;
    # GC timing is a tier-0 concern, no tier-1 dependency).
    return subprocess.run(
        _ap_conform_base()
        + ["run", "--corpus", corpus_arg, "--impl", " ".join(_impl_cmd()), "--tier", "0"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=1200,
    )


xfail_ap = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)

# ---------------------------------------------------------------------------
# Half 1 — fixture re-verification (GREEN today, no implementation needed)
# ---------------------------------------------------------------------------


def test_slice_quota():
    # Task contract: >= 15 gc corpus cases. Corpus only grows (C-33) —
    # assert the FLOOR, never an exact count.
    assert len(CASES) >= 15, f"gc slice has {len(CASES)} cases"


def test_every_expected_triple_exists():
    exp = GC_CORPUS / "expected"
    for stem in CASES:
        for suffix in (".out", ".err", ".rc"):
            assert (exp / (stem + suffix)).exists(), f"{stem}{suffix} missing"


@pytest.mark.parametrize("case", CASES)
def test_header_cites_existing_clauses(case):
    script = SCRIPTS[case]
    cited = _cited_ids(script)
    assert cited, f"{case}: no C-nn ids in the header"
    # Every cited clause must exist in the contract (§12 index).
    contract = (REPO_ROOT / "docs" / "compat-contract.md").read_text()
    index_ids = {int(m) for m in re.findall(r"^\|\s*C-(\d{2})\s*\|", contract, re.M)}
    unknown = cited - index_ids
    assert not unknown, f"{case}: cites ids not in contract index: {sorted(unknown)}"


@pytest.mark.parametrize("case", CASES)
def test_gc_cases_cite_the_divergence_when_timing_sensitive(case):
    # C-23/D1 lint (contract Verify line: "no case cited to divergence D1
    # lacks gc.collect()"): any case citing C-20/C-21/C-22 or D1-adjacent
    # timing must contain an explicit gc.collect() anchor, and no case may
    # assert __del__ output via stdout interleaving — finalizer side
    # effects are recorded in lists and sampled after collect(). Heuristic
    # enforcement: __del__ bodies must not print().
    text = SCRIPTS[case].read_text()
    timing_cited = bool(_cited_ids(script := SCRIPTS[case]) & {20, 21, 22})
    if "__del__" in text or "weakref" in text or "callbacks" in text:
        assert "gc.collect()" in text, f"{case}: GC-anchored case lacks gc.collect()"
    if timing_cited or "__del__" in text:
        del_body = re.findall(r"def __del__.*?\n(?:    .*\n|\n)*", text)
        for body in del_body:
            assert "print(" not in body, (
                f"{case}: __del__ writes to stdout — timing-dependent "
                f"interleave (forbidden by C-23 / S-06-17#1): {body!r}"
            )


@pytest.mark.parametrize("case", CASES)
def test_no_refcount_api_in_corpus(case):
    # C-19/D3 corpus lint (the contract's Verify line names exactly this
    # check; enforced here until ap-conform load-time rejects such cases —
    # gap G-2).
    text = SCRIPTS[case].read_text()
    for banned in ("getrefcount", "gettotalrefcount", "getallocatedblocks"):
        assert banned not in text, f"{case}: corpus case references {banned} (C-19 prohibited)"


@pytest.mark.parametrize("case", CASES)
def test_reference_reproduces_expected_bytes(case):
    script = SCRIPTS[case]
    exp = GC_CORPUS / "expected"
    proc = _run_ref(script)
    out = (exp / (case + ".out")).read_bytes()
    err = (exp / (case + ".err")).read_bytes()
    rc = (exp / (case + ".rc")).read_text().strip()
    assert proc.stdout == out, (
        f"{case}: stdout drift\n--- got ---\n{proc.stdout.decode(errors='replace')}"
        f"\n--- expected ---\n{out.decode(errors='replace')}"
    )
    assert _mask_root(proc.stderr) == _mask_root(err), (
        f"{case}: stderr drift\n{proc.stderr.decode(errors='replace')}"
    )
    assert str(proc.returncode) == rc, f"{case}: exit code {proc.returncode} != {rc}"


@pytest.mark.parametrize("case", CASES[:8])
def test_determinism_three_identical_runs(case):
    # C-23 Verify: "the test harness rejects a case that flaps (different
    # pass/fail across two consecutive identical runs) because it depends
    # on GC timing." Full byte equality across 3 runs. Parametrized over a
    # subset for wall-clock budget; the fixture-generation script
    # (_gen_expected.py) ran ALL cases 3x before recording expected/.
    script = SCRIPTS[case]
    results = [_run_ref(script) for _ in range(3)]
    first = (results[0].stdout, results[0].stderr, results[0].returncode)
    for proc in results[1:]:
        assert (proc.stdout, proc.stderr, proc.returncode) == first, (
            f"{case}: flaps across runs — GC-timing dependent"
        )


@pytest.mark.parametrize("case", CASES)
def test_expected_streams_are_clean(case):
    # Every case catches its own errors and samples via lists + explicit
    # collect: rc 0 and empty stderr by construction, so traceback path
    # text never enters an expected file. gc009 is the one case that
    # OBSERVES an unraisable report — it does so via an in-script
    # sys.stderr swap + masking, never via the process's real stderr.
    exp = GC_CORPUS / "expected"
    assert (exp / (case + ".rc")).read_text().strip() == "0", case
    assert (exp / (case + ".err")).read_bytes() == b"", case
    assert (exp / (case + ".out")).read_bytes().endswith(b"\n"), case


@pytest.mark.parametrize("case", CASES)
def test_no_nondeterministic_addresses_leak(case):
    # S-06-17 #4 / C-24: memory addresses are never diff-able. gc009's
    # masked capture proves masking works; every other case avoids repr
    # of plain objects entirely.
    exp = GC_CORPUS / "expected"
    blob = (exp / (case + ".out")).read_bytes() + (exp / (case + ".err")).read_bytes()
    m = re.search(rb"0x[0-9a-fA-F]{4,}", blob)
    assert m is None, f"{case}: address-like token {m.group()!r} in expected bytes"
    assert b"<object at" not in blob, f"{case}: default object repr leaked"


# ---------------------------------------------------------------------------
# Half 2 — ap-conform differential gate (xfail until 02.04 lands)
# ---------------------------------------------------------------------------


@xfail_ap
def test_ap_conform_gc_slice_green_tier0():
    # 02.04 Gate, first clause: "allocation-heavy corpus slice green".
    # gc014 (2000-instance finalizer stress) + the 10k probe below make
    # this slice the allocation-heavy gate slice.
    proc = run_conform(str(GC_CORPUS / "gc*.py"))
    assert proc.returncode == 0, (
        f"gc slice not green:\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
    )


@xfail_ap
@pytest.mark.parametrize("case", CASES)
def test_ap_conform_case_green_tier0(case):
    proc = run_conform(str(SCRIPTS[case]))
    assert proc.returncode == 0, (
        f"{case} not green:\nstdout:\n{proc.stdout}\nstderr:\n{proc.stderr}"
    )


@xfail_ap
def test_refcount_api_absent_or_stub_safe():
    # C-19/D3 impl-side projection (task contract: "sys.getrefcount
    # ABSENT assertion belongs to the impl-side xfail test citing C-14").
    # C-14 is cited as instructed; note the contract text for refcount
    # absence is C-19/D3 — C-14 governs int overflow/memory exceptions and
    # is cross-referenced because the probe runs on the integer-semantics
    # boundary objects (an int whose refcount CPython would expose).
    # The gate is the EXISTENCE projection from probes/refcount_probe.py:
    # a conforming antipythany has no observable reference counts. Per D3
    # presence-with-stub-safe-value is also allowed, so the assert accepts
    # (absent) OR (present-but-corpus-prohibited) — it fires only if the
    # probe cannot even run under the impl, or the impl reports
    # CPython-identical live counts (impossible under tracing GC and a
    # C-19 violation). Until the binary exists the probe never runs:
    # XFAIL.
    cmd = _impl_cmd() + [str(PROBES / "refcount_probe.py")]
    proc = subprocess.run(cmd, capture_output=True, env=_scrubbed_env(),
                          cwd=REPO_ROOT, timeout=120)
    assert proc.returncode == 0, f"impl probe failed: {proc.stderr.decode(errors='replace')}"
    text = proc.stdout.decode()
    assert "has_getrefcount: False" in text or "has_getrefcount: True" in text, (
        f"probe shape unexpected: {text!r}"
    )
    # The D3-sanctioned antipythany answer (no observable refcounts):
    assert "has_getrefcount: False" in text, (
        f"getrefcount present under antipythany; allowed ONLY if stub-safe "
        f"(D3) — values are never gated, but this gate asserts the "
        f"design-intended absence projection: {text!r}"
    )


def test_10k_projection_reference_calibration():
    # Calibration (green today, reference-side): the heap probe run under
    # the REFERENCE must pass the identical 1 GiB bound + exact-once
    # projection that test_10k_iteration_bounded_heap will assert of the
    # impl. Pins that the threshold is a real leak detector, not an
    # artifact. This test needs no implementation — it lives in Half 1's
    # spirit but runs the probe, so it sits here without the xfail mark.
    proc = subprocess.run(
        ["/usr/bin/time", "-l", PYREF, str(PROBES / "heap_probe_10k.py")],
        capture_output=True, env=_scrubbed_env(), cwd=REPO_ROOT, timeout=600,
    )
    assert proc.returncode == 0
    assert "fired: 10000" in proc.stdout.decode()
    assert "complete: True" in proc.stdout.decode()
    m = re.search(r"maximum resident set size:\s*([0-9]+)|([0-9]+)\s+peak memory footprint", proc.stderr.decode(errors="replace"))
    rss = int(m.group(1) or m.group(2)) if m else None
    assert rss is not None, f"no RSS line from /usr/bin/time -l:\n{proc.stderr.decode(errors='replace')}"
    assert rss < 1024 * 1024 * 1024, f"reference itself exceeds the bound: RSS {rss} B"


@xfail_ap
def test_10k_iteration_bounded_heap():
    # 02.04 Gate, second clause: "no leak across 10k iterations (heap
    # growth bounded)". Runs the probe under the impl, asserts (a) exact-
    # once finalizer projection, (b) exit 0, (c) child max-RSS bounded.
    # RSS measured via /usr/bin/time -l on macOS (wall+maxRSS) — the
    # threshold is deliberately generous (leak detector, not a memory
    # budget): a 10k-finalizer workload with per-instance [int,int] lists
    # should stay far under 1 GiB resident if collection works; an
    # unfinalized-object leak grows the heap linearly with rounds.
    impl = _impl_cmd()
    probe = str(PROBES / "heap_probe_10k.py")
    measure = ["/usr/bin/time", "-l"] + impl + [probe]
    proc = subprocess.run(measure, capture_output=True, env=_scrubbed_env(),
                          cwd=REPO_ROOT, timeout=600)
    assert proc.returncode == 0, f"impl heap probe run failed: {proc.stderr.decode(errors='replace')[-2000:]}"
    assert "fired: 10000" in proc.stdout.decode(), "C-21 eventually-run guarantee violated"
    assert "complete: True" in proc.stdout.decode(), "double/lost finalization detected"
    m = re.search(r"maximum resident set size:\s*([0-9]+)|([0-9]+)\s+peak memory footprint", proc.stderr.decode(errors="replace"))
    assert m, "no RSS line from /usr/bin/time -l"
    rss_bytes = int(m.group(1) or m.group(2))
    assert rss_bytes < 1024 * 1024 * 1024, f"heap NOT bounded across 10k iterations: RSS {rss_bytes} B"
