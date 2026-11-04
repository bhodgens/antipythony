# T-16 — hybrid artifact (gates implementation leaf 04.05 hybrid artifact).
#
# Authorship: written from the plan/contract documents ONLY
# (plans/antipythony/04-packaging-ecosystem/orchestrator.md 04.05 Expose/Gate +
# the CLI verb surface, plans/antipythony/03-wasm-backend/orchestrator.md
# semantics-invariance rule + ap-host runner form 'ap-host run' (03.02 harness
# flag) + the D4 tier-1 note (03.04), docs/compat-contract.md C-24/C-27/C-29/C-30/C-31
# + §9 D4, plans/antipythony/master.md (Two-artifact decision, test-tree
# protocol), plans/antipythony/tests/README.md, exemplars tests/t09_binary/ +
# tests/t08_tier1/ cross-mode-equality pattern). No implementation source under
# crates/ was read. Per-test citations and CONTRACT-GAPS:
# plans/antipythony/tests/T-16-hybrid-artifact.md.
#
# What is gated (04.05, verbatim Expose): "Native binary with the wasm module
# embedded (payload section / include_bytes at link time); `app --emit-wasm`
# extracts; embedded ap-host execution path behind a flag." Gate: "one binary
# passes native conformance slice AND its extracted wasm passes the wasm slice."
#
# Design — THREE-WAY BYTE IDENTITY (cite C-31 + C-30; see doc §Design for the
# C-30/C-31 citation note, G-1): for each pinned corpus program, the same
# (stdout, stderr, rc) triple must hold on every execution leg, compared as
# PAIRWISE equalities exactly like T-08's cross-mode pattern:
#   native  = `antipythony build app.py -o app`          -> ./app
#   hybrid  = `antipythony build --hybrid app.py -o app` -> ./app (native + payload)
#   wasm    = `antipythony build --wasm app.py` output, and the .wasm EXTRACTED
#             from the hybrid via `app --emit-wasm`, both executed by `ap-host run`.
# Per-program artifacts ride on the EXISTING monotonic corpus (C-33; read-only):
# one values case + one classes case, both inside T-09's pinned 30-case slice.
#
# Structure:
#   GREEN NOW   — fixture validation only: the two pinned corpus programs exist,
#                 their expected triples re-measure green against the reference
#                 CPython, headers cite clause ids, and the triples are clean
#                 exits (rc 0, empty stderr — the C-30 anchor every leg must hit).
#   XFAIL (04.05) — everything antipythony-side: hybrid build seam, native-green
#                 run of the hybrid binary, `--emit-wasm` extraction, embedded-
#                 payload byte identity vs a fresh `build --wasm` artifact
#                 (separately-marked repro gap, G-3), the pairwise legs
#                 (native↔hybrid, native↔wasm, wasm-build↔extracted-wasm under
#                 ap-host), the wasm-slice check (extracted wasm vs pinned triple
#                 and via ap-conform), `--emit-wasm` on a NON-hybrid native binary
#                 failing with a clean diagnostic (G-4), the native-flag/argv
#                 acceptance paths (G-5), the aggregate three-way lattice, and
#                 PATH isolation of the hybrid binary.
#   SPEC-07 NOTE — docs/spec/07-wasm-repr.md (leaf 03.01) does not exist yet, so
#   every wasm-side assertion is limited to C-31/C-30 BYTE IDENTITY of the three
#   conformance channels, which needs no representation detail. No
#   representation-level assertion is invented anywhere (doc §Spec-07
#   dependency + G-2).

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
CORPUS = REPO_ROOT / "tests" / "corpus"

# Reference CPython (C-01: pinned 3.14.0; recorded/verified here on 3.14.7 —
# provisional, same open item as T-09 GAP-4 / T-06 GAP-7).
# Reference discovery: unified env+PATH ladder (robustness-ci 01.01); see
# tests/tools/_ref_python.py for the AP_REQUIRE_REFERENCE=1 policy.
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
from _ref_python import reference_or_skip  # noqa: E402

RUN_TIMEOUT = int(os.environ.get("APX_T16_TIMEOUT", "300"))

XFAIL_0405 = pytest.mark.xfail(
    strict=False,
    reason="awaits 04.05 hybrid artifact (+ 04.04 ap-host runner for wasm legs); "
    "spec 07 pending: wasm-side asserts are byte-identity only, per C-31/C-30",
)
XFAIL_REPRO = pytest.mark.xfail(
    strict=False,
    reason="G-3: embedded-payload byte identity vs a fresh `build --wasm` output "
    "is unpinned by the docs — the 04.02 reproducibility gate text covers NATIVE "
    "build output; wasm payload reproducibility awaits a decision",
)

# ---------------------------------------------------------------------------
# Pinned program pair (task design: one values case + one classes case from the
# EXISTING monotonic corpus; both are inside T-09's SLICE_30, so their bytes are
# already slice-blessed anchors). Paths frozen here; the manifest test keeps
# them honest. Corpus is NEVER modified by this leaf (C-33).
# ---------------------------------------------------------------------------
CASES = [
    CORPUS / "values" / "V003-floordiv-mod-signs.py",   # S-01-3/S-01-4, C-12
    CORPUS / "classes" / "c001_class_basics.py",        # S-03-01, S-03-04
]

# CLI seam defaults (all overridable; gap G-5 documents each non-pinned token):
HYBRID_BUILD_FLAGS = shlex.split(os.environ.get("APX_HYBRID_BUILD_FLAGS", "--hybrid"))
WASM_BUILD_FLAGS = shlex.split(os.environ.get("APX_WASM_BUILD_FLAGS", "--wasm"))
EMIT_WASM_ARGV = shlex.split(os.environ.get("APX_EMIT_WASM_ARGV", "--emit-wasm"))
EMBEDDED_HOST_FLAG = os.environ.get("APX_HYBRID_EXEC_FLAG", "--wasm")


def _clean_env(path_value=None) -> dict:
    env = {k: os.environ[k] for k in ("HOME", "LANG", "TMPDIR") if k in os.environ}
    env["PATH"] = path_value if path_value is not None else os.environ.get("PATH", "")
    return env


def _find_binary(name: str, env_var: str) -> str:
    """Documented resolution order (mirrors T-09): env override -> PATH ->
    target/{debug,release}/<name>. FileNotFoundError until the artifact exists,
    which lands inside the XFAIL envelope for antipythony-side tests."""
    override = os.environ.get(env_var)
    if override:
        return shlex.split(override)[0]
    found = shutil.which(name)
    if found:
        return found
    for profile in ("debug", "release"):
        cand = REPO_ROOT / "target" / profile / name
        if cand.exists():
            return str(cand)
    raise FileNotFoundError(f"{name} not built/found (set {env_var}=<cmdline> to override)")


def _cli_cmd() -> list:
    """The `antipythony` driver (build verbs per the Plan 04 surface table)."""
    override = os.environ.get("APX_CLI")
    if override:
        return shlex.split(override)
    return [_find_binary("antipythony", "APX_CLI")]


def _ap_host_cmd() -> list:
    """ap-host runner base command. CLI shape from 03.02: 'ap-host run'
    (harness flag `ap-conform run --impl 'ap-host run'`); 04.04 also shows a
    standalone `./ap-host app.wasm` form — 'run' subcommand is the cited shape
    for the gate (G-5 note)."""
    override = os.environ.get("APX_APHOST_CMD")
    if override:
        return shlex.split(override)
    return [_find_binary("ap-host", "APX_APHOST"), "run"]


def _ap_conform_base() -> list:
    override = os.environ.get("AP_CONFORM")
    if override:
        return shlex.split(override)
    return [_find_binary("ap-conform", "AP_CONFORM")]


def _run(cmd: list, cwd=None, path_env=None) -> subprocess.CompletedProcess:
    return subprocess.run(
        cmd, capture_output=True, cwd=cwd or REPO_ROOT,
        env=_clean_env(path_env), timeout=RUN_TIMEOUT,
    )


def _build(script: Path, tag: str, extra_flags=()) -> Path:
    """`antipythony build [flags] <script> -o <tmp>/<tag>` (04.05/02.06 Expose);
    a zero-exit build with no artifact is a seam failure (same rule as T-09 G-6),
    never a vacuous pass."""
    root = Path(tempfile.mkdtemp(prefix=f"t16_{tag}_"))
    out = root / tag
    proc = _run(_cli_cmd() + ["build", *extra_flags, str(script), "-o", str(out)])
    assert proc.returncode == 0, (
        f"build failed rc={proc.returncode}\n"
        f"stdout:\n{proc.stdout.decode(errors='replace')}\n"
        f"stderr:\n{proc.stderr.decode(errors='replace')}"
    )
    assert out.exists(), f"build exited 0 but produced no artifact at {out}"
    return out


def _build_native(script: Path, tag: str) -> Path:
    app = _build(script, tag)
    assert os.access(app, os.X_OK), f"native artifact {app} is not executable"
    return app


def _build_hybrid(script: Path, tag: str) -> Path:
    app = _build(script, tag, extra_flags=HYBRID_BUILD_FLAGS)
    assert os.access(app, os.X_OK), f"hybrid artifact {app} is not executable"
    return app


def _build_wasm(script: Path, tag: str) -> Path:
    mod = _build(script, tag + ".wasm", extra_flags=WASM_BUILD_FLAGS)
    assert mod.read_bytes()[:4] == b"\x00asm", f"{mod} lacks the wasm magic header"
    return mod


def _emit_wasm(app: Path) -> Path:
    """Run `app --emit-wasm` in an EMPTY temp dir; the leaf text pins the flag
    name but not the output location, so: rc 0 + exactly one *.wasm appearing in
    the working dir is the operative contract (gap G-5)."""
    workdir = Path(tempfile.mkdtemp(prefix="t16_emit_"))
    proc = _run([str(app), *EMIT_WASM_ARGV], cwd=workdir)
    assert proc.returncode == 0, (
        f"--emit-wasm rc={proc.returncode}\n"
        f"stdout:\n{proc.stdout.decode(errors='replace')}\n"
        f"stderr:\n{proc.stderr.decode(errors='replace')}"
    )
    produced = sorted(workdir.rglob("*.wasm"))
    assert len(produced) == 1, (
        f"--emit-wasm produced {len(produced)} .wasm files in {workdir}: {produced}"
    )
    return produced[0]


def _triple(proc: subprocess.CompletedProcess) -> tuple:
    """The whole observable contract (C-24/C-27): stdout bytes, stderr bytes,
    exit code — no normalization. Signal death maps to 128+signo (POSIX status,
    same convention as T-09)."""
    rc = proc.returncode if proc.returncode >= 0 else 128 + abs(proc.returncode)
    return (proc.stdout, proc.stderr, rc)


def _pinned_triple(script: Path) -> tuple:
    """Expected triple from the corpus, supporting BOTH recorded naming
    conventions (.stdout/.stderr/.exit for values, .out/.err/.rc for classes —
    exactly the dual lookup T-08's cross-mode test uses)."""
    exp = script.parent / "expected"
    for suffixes in ((".stdout", ".stderr", ".exit"), (".out", ".err", ".rc")):
        trio = [exp / (script.stem + s) for s in suffixes]
        if all(t.is_file() for t in trio):
            return (trio[0].read_bytes(), trio[1].read_bytes(),
                    int(trio[2].read_text().strip()), trio)
    raise AssertionError(f"no complete expected triple for {script}")


def _case_id(p: Path) -> str:
    return p.relative_to(REPO_ROOT).as_posix()


# ---------------------------------------------------------------------------
# HALF 1 — GREEN NOW: fixture validation. These test the corpus and the
# reference, not antipythony; they must STAY green (a break = C-01/GAP drift).
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_pinned_case_and_expected_triple_exist(script):
    """Manifest honesty: the pinned program exists under tests/corpus/ and its
    complete expected triple ships next to it (C-33: corpus is read-only
    reference data for this leaf — nothing here is modified)."""
    assert script.is_file(), f"missing corpus program {script}"
    assert script.parent.name in ("values", "classes"), (
        "task design pins one values case + one classes case"
    )
    _pinned_triple(script)


@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_reference_reproduces_expected_bytes(script):
    """The pinned triple is the semantics anchor (C-01/C-02/C-27): re-running
    the program under the reference build reproduces every channel byte-for-
    byte, so drift is loud HERE and never blamed on a 04.05 artifact."""
    proc = _run([reference_or_skip(), str(script)], cwd=script.parent)
    exp_out, exp_err, exp_rc, _ = _pinned_triple(script)
    assert proc.stdout == exp_out, (
        f"{script.name}: stdout drift\n got={proc.stdout!r}\n exp={exp_out!r}"
    )
    assert proc.stderr == exp_err, f"{script.name}: stderr drift {proc.stderr!r}"
    assert _triple(proc)[2] == exp_rc, f"{script.name}: rc {_triple(proc)[2]} != pinned {exp_rc}"


@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_case_header_cites_clause_ids(script):
    """tests/README.md corpus rule: every corpus case header cites the clause
    ids it exercises — the hybrid legs gate programs that are traceable."""
    text = script.read_text(encoding="utf-8")
    cited = re.findall(r"\b(?:C-\d{2}|S-\d\d-\d+)\b", text)
    assert cited, f"{script.name}: no clause citations in file"


@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_pinned_triple_is_clean_exit_anchor(script):
    """C-30: exit code 0 is reserved for clean exit. Both pinned programs
    complete cleanly (rc 0, empty stderr, newline-terminated stdout) — so every
    leg of the three-way lattice MUST land on rc 0 with empty stderr; any leg
    that prints a traceback or exits nonzero is a violation, not a washout."""
    exp_out, exp_err, exp_rc, _ = _pinned_triple(script)
    assert exp_rc == 0, f"{script.name}: pinned rc != 0 (C-30 anchor broken)"
    assert exp_err == b"", f"{script.name}: pinned stderr not empty (C-25/C-29)"
    assert exp_out.endswith(b"\n"), f"{script.name}: stdout not newline-terminated (C-27)"


def test_case_pair_is_one_values_one_classes():
    """Composition of the program pair (task design): distinct slices, each
    present exactly once — values pins C-12 arithmetic bytes, classes pins
    S-03-01 type machinery; together they stress two lowering paths of both
    backends (04.05 Gate's 'native conformance slice' / 'wasm slice' minima)."""
    slices = [p.parent.name for p in CASES]
    assert slices.count("values") == 1 and slices.count("classes") == 1
    assert len({p.stem for p in CASES}) == 2


# ---------------------------------------------------------------------------
# HALF 2 — 04.05 GATE (xfail, strict=False). The HYBRID binary is the artifact.
# ---------------------------------------------------------------------------


@XFAIL_0405
def test_hybrid_build_produces_executable():
    """04.05 Expose: `antipythony build --hybrid` -> one executable native
    binary with the wasm payload embedded (master.md: the hybrid is the third
    distribution shape over 'native binary + separate .wasm')."""
    app = _build_hybrid(CASES[0], "hyb_app")
    assert app.exists() and os.access(app, os.X_OK)


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_hybrid_binary_native_green(script):
    """04.05 Gate half 1 verbatim ('one binary passes native conformance
    slice'): the hybrid binary executed plainly reproduces the pinned CPython
    triple on all three channels (C-27; C-31 — hybrid-vs-native is behaviorally
    invisible; rc 0 per the C-30 anchor)."""
    app = _build_hybrid(script, "hyb_native")
    proc = _run([str(app)])
    exp = _triple(_run([reference_or_skip(), str(script)], cwd=script.parent))
    assert proc.stdout == exp[0], f"{script.name}: hybrid native stdout drift"
    assert proc.stderr == exp[1], f"{script.name}: hybrid native stderr drift (C-25)"
    assert _triple(proc)[2] == exp[2], f"{script.name}: hybrid native rc != pinned (C-30)"


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_emit_wasm_extracts_module(script):
    """04.05 Expose: `app --emit-wasm` extracts the embedded module — the run
    exits 0 and produces exactly one .wasm (magic \\x00asm) in the working dir.
    Output LOCATION is unpinned (G-5); empty-cwd discovery is the operative read."""
    app = _build_hybrid(script, "hyb_emit")
    mod = _emit_wasm(app)
    assert mod.read_bytes()[:4] == b"\x00asm", f"extracted file {mod} is not a wasm module"


@XFAIL_REPRO
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_embedded_payload_byte_matches_wasm_build(script):
    """Reproducibility of the EMBEDDED PAYLOAD: the extracted .wasm must be
    byte-identical to a separately built `antipythony build --wasm` artifact of
    the same source — otherwise the hybrid silently ships a different program
    than the portable distribution. GAP G-3: no doc pins wasm-output
    reproducibility (04.02's gate text covers native build output only), so
    this is its own xfail envelope, not the 04.05 blocker."""
    app = _build_hybrid(script, "hyb_repro")
    extracted = _emit_wasm(app)
    standalone = _build_wasm(script, "wasm_standalone")
    assert extracted.read_bytes() == standalone.read_bytes(), (
        f"{script.name}: embedded payload != fresh `build --wasm` output "
        f"(sizes {extracted.stat().st_size} vs {standalone.stat().st_size})"
    )


# --- the three-way pairwise legs (C-31 rule, cited as C-30 per the plan tree's
# --- shorthand; see doc G-1 for the citation-alignment note). Structure
# --- mirrors T-08's cross-mode equality tests: each pair compared directly,
# --- plus comparison against the pinned reference where the triple ships.


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_pair_native_vs_hybrid(script):
    """Pairwise leg N↔H: plain native binary (02.06 `build`) and hybrid binary
    (`build --hybrid`) agree byte-for-byte on all three channels (C-31:
    AoT-vs-hybrid difference invisible; C-30: clean rc 0 both legs)."""
    native = _triple(_run([str(_build_native(script, "pair_native"))]))
    hybrid = _triple(_run([str(_build_hybrid(script, "pair_hybrid"))]))
    assert hybrid == native, (
        f"{script.name}: hybrid triple {hybrid[2]}/{len(hybrid[0])}b != "
        f"native triple {native[2]}/{len(native[0])}b (C-31)"
    )


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_pair_native_vs_wasm_under_ap_host(script):
    """Pairwise leg N↔W: the native binary vs `build --wasm` output executed by
    ap-host (`ap-host run`, the 03.02 harness shape) — the Plan 03 semantics-
    invariance rule / C-31's explicit 'native and wasm artifacts MUST produce
    byte-identical stdout/stderr/exit code' sentence. wasm has no tier-1 in v1
    (D4): timing-only, never a byte difference, so it is not a washout here."""
    native = _triple(_run([str(_build_native(script, "nw_native"))]))
    mod = _build_wasm(script, "nw_wasm")
    wasm = _triple(_run(_ap_host_cmd() + [str(mod)]))
    assert wasm == native, (
        f"{script.name}: wasm(ap-host) triple != native triple (C-31, D4 timing-only)"
    )


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_pair_wasm_build_vs_extracted_under_ap_host(script):
    """Pairwise leg W↔H(wasm): the standalone `build --wasm` artifact and the
    hybrid's EXTRACTED module, both run under ap-host, agree on all three
    channels. Transitive with the repro test above this pins that users get
    ONE program whichever distribution shape they pick (master.md two-artifact
    + hybrid decision; C-31)."""
    standalone = _triple(_run(_ap_host_cmd() + [str(_build_wasm(script, "hw_standalone"))]))
    extracted = _triple(_run(_ap_host_cmd() + [str(_emit_wasm(_build_hybrid(script, "hw_hybrid")))]))
    assert extracted == standalone, (
        f"{script.name}: extracted-wasm run triple != standalone-wasm run triple (C-31)"
    )


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_extracted_wasm_passes_wasm_slice(script):
    """04.05 Gate half 2 verbatim ('its extracted wasm passes the wasm slice'),
    asserted directly: `ap-host run <extracted.wasm>` reproduces the PINNED
    reference triple byte-for-byte (C-27/C-30). DESIGN-PENDING caveat:
    docs/spec/07-wasm-repr.md does not exist yet (leaf 03.01 unimplemented), so
    this gate half asserts BYTE IDENTITY ONLY — no representation-level
    assertion is made or invented until spec 07 lands (doc §Spec-07 dependency)."""
    extracted = _emit_wasm(_build_hybrid(script, "wslice_hybrid"))
    proc = _run(_ap_host_cmd() + [str(extracted)])
    exp_out, exp_err, exp_rc, _ = _pinned_triple(script)
    assert proc.stdout == exp_out, f"{script.name}: extracted-wasm stdout != reference (C-27/C-31)"
    assert proc.stderr == exp_err, f"{script.name}: extracted-wasm stderr != reference (C-25)"
    assert _triple(proc)[2] == exp_rc, f"{script.name}: extracted-wasm rc != pinned 0 (C-30)"


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_ap_conform_wasm_slice_via_ap_host_impl(script):
    """Gate half 2 in harness form: `ap-conform run --corpus <case> --impl
    'ap-host run'` — the exact --impl shape 03.02 documents. GAP G-6: no doc
    read by this author says whether ap-conform compiles the .py case to wasm
    before invoking the impl or expects pre-built modules; if the harness form
    is unimplementable today the direct byte check above still pins the gate
    either way."""
    impl = " ".join(_ap_host_cmd())
    cmd = _ap_conform_base() + ["run", "--corpus", str(script), "--impl", impl]
    proc = _run(cmd)
    assert proc.returncode == 0, (
        f"{_case_id(script)}: ap-conform-vs-ap-host failed rc={proc.returncode}\n"
        f"{proc.stdout.decode(errors='replace')}\n{proc.stderr.decode(errors='replace')}"
    )


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_three_way_lattice_aggregate(script):
    """The 04.05 gate as one assertion: NATIVE run == HYBRID run ==
    ap-host(EXTRACTED wasm) == ap-host(STANDALONE wasm) == pinned reference
    triple — the three-way byte identity (C-31 rule cited in the plan tree as
    the C-30 native-vs-wasm rule; all three channels, C-24/C-27; clean rc 0,
    C-30). Pairwise legs above localize a failure; this is the CI one-liner."""
    pinned = _pinned_triple(script)[:3]
    native = _triple(_run([str(_build_native(script, "lat_native"))]))
    hybrid_app = _build_hybrid(script, "lat_hybrid")
    hybrid = _triple(_run([str(hybrid_app)]))
    extracted = _emit_wasm(hybrid_app)
    ext_run = _triple(_run(_ap_host_cmd() + [str(extracted)]))
    standalone_run = _triple(_run(_ap_host_cmd() + [str(_build_wasm(script, "lat_wasm"))]))
    for label, got in (("native", native), ("hybrid", hybrid),
                       ("extracted-wasm", ext_run), ("standalone-wasm", standalone_run)):
        assert got == pinned, f"{script.name}: {label} leg differs from pinned triple (C-31)"
        assert got == native, f"{script.name}: {label} leg differs from native leg (C-31)"


@XFAIL_0405
def test_emit_wasm_rejected_on_plain_native_binary():
    """`--emit-wasm` is a HYBRID capability: on a plain `build` (non-hybrid)
    native binary the flag MUST fail cleanly — nonzero exit (not a signal death:
    a panic-on-unknown-flag is not a 'clean diagnostic'), non-empty stderr, and
    NO .wasm produced (C-11's totality spirit: no half-artifact). STRICTEST
    reading of the leaf text, which attaches the flag to the hybrid artifact
    only — GAP G-4: if the leaf text is unimplementable (implementers decide a
    plain binary accepts and no-ops the flag), the fix is a leaf-text
    amendment, not a silent test weakening."""
    script = CASES[0]
    app = _build_native(script, "plain_native")
    workdir = Path(tempfile.mkdtemp(prefix="t16_plain_"))
    proc = _run([str(app), *EMIT_WASM_ARGV], cwd=workdir)
    assert proc.returncode != 0, "--emit-wasm on a non-hybrid binary exited 0"
    assert proc.returncode > 0, f"clean nonzero failure expected, got signal death {proc.returncode}"
    assert proc.stderr.strip() != b"", "--emit-wasm refusal produced no stderr diagnostic"
    assert not list(workdir.rglob("*.wasm")), "non-hybrid binary emitted a .wasm anyway"


@XFAIL_0405
def test_hybrid_binary_accepts_native_argv_passthrough():
    """Task design (v): the hybrid binary is a NATIVE binary first — the plain
    execution path accepts program argv exactly like 02.06's binary (x011-style
    passthrough), with no flag consumption of trailing args. Program pinned here
    ignores argv, so the check is: args accepted AND the pinned triple
    unchanged (C-27/C-31). The argv-echo form is cross-referenced to T-09's
    `test_argv_passthrough` — this leg pins that --hybrid didn't break it."""
    script = CASES[1]  # c001_class_basics.py: argv-ignoring, deterministic
    app = _build_hybrid(script, "argv_hybrid")
    proc = _run([str(app), "alpha", "2", "--not-a-hybrid-flag"])
    exp_out, exp_err, exp_rc, _ = _pinned_triple(script)
    assert proc.stdout == exp_out, f"argv passthrough changed stdout (C-31 invisibility)"
    assert proc.stderr == exp_err and _triple(proc)[2] == exp_rc


@XFAIL_0405
@pytest.mark.parametrize("script", CASES, ids=[_case_id(c) for c in CASES])
def test_embedded_host_exec_flag_same_triple(script):
    """04.05 Expose, third phrase: 'embedded ap-host execution path behind a
    flag' — running the hybrid binary WITH that flag executes the embedded wasm
    through the embedded host, and MUST print exactly the same triple as the
    plain native path (C-31 artifact/execution-path invisibility; D4 says the
    wasm leg's tier-1 absence is timing-only, never bytes). GAP G-5: no document
    pins the flag's spelling; default `--wasm`, override APX_HYBRID_EXEC_FLAG."""
    app = _build_hybrid(script, "eh_hybrid")
    plain = _triple(_run([str(app)]))
    via_flag = _triple(_run([str(app), EMBEDDED_HOST_FLAG]))
    assert via_flag == plain, (
        f"{script.name}: embedded-host flag path triple != native path triple (C-31)"
    )


@XFAIL_0405
def test_hybrid_binary_runs_without_python_on_path():
    """The hybrid is still a single static native binary (master.md artifact
    decision; Go-style UX): the native execution path works with
    PATH=/usr/bin:/bin, which contains no python3.14 — the embedded payload and
    embedded host must not smuggle an interpreter dependency back in. Same
    standing rule T-09 records as G-7 (implied, not written)."""
    iso_path = "/usr/bin:/bin"
    assert shutil.which("python3.14", path=iso_path) is None, (
        "isolation PATH unexpectedly contains python3.14"
    )
    script = CASES[0]
    app = _build_hybrid(script, "iso_hybrid")
    proc = _run([str(app)], path_env=iso_path)
    exp_out, exp_err, exp_rc, _ = _pinned_triple(script)
    assert (proc.stdout, proc.stderr, _triple(proc)[2]) == (exp_out, exp_err, exp_rc), (
        "hybrid binary not self-sufficient under PATH isolation (02.06 UX inherited by 04.05)"
    )
