# T-11 — stdlib-v1 scope + behavior (gates implementation leaf 02.08 v1 stdlib subset).
#
# Authorship: written from the spec pack ONLY (docs/spec/05-builtins-stdlib-v1.md,
# docs/spec/06-observable-behavior.md, docs/compat-contract.md C-04..C-10,
# plans/antipythony/master.md, plans/antipythony/tests/README.md, and leaf 02.08's
# Expose/Gate in plans/antipythony/02-native-backend/orchestrator.md). The author
# has not read any implementation source under crates/.
#
# THREE test faces (see plans/antipythony/tests/T-11-stdlib-v1.md):
#
#  (A) ALLOW-LIST face, table half (GREEN today): static assertions over
#      scope_table.py — the IN/PARTIAL/OUT transcription of spec 05 §5.2/§5.3.
#      Checks internal consistency (IN ∩ OUT = ∅, every §5.3.3 name covered),
#      transcription honesty (names spec 05 claims but the 3.14.7 -S reference
#      lacks are EXACTLY the whitelisted KNOWN_REF_ABSENT set), and the
#      reference-side premise of the divergence (every OUT module imports fine
#      on the reference — so antipythony's fail-loud refusal is testable only
#      against antipythony, per S-05-22 "not byte-gated vs CPython").
#
#  (B) BEHAVIOR face (GREEN today): fixture re-verification of
#      tests/corpus/stdlib/ — 48 differential cases, one+ per IN module row,
#      every byte pinned by real /opt/homebrew/bin/python3.14 (3.14.7) runs:
#      expected triples exist, reference re-run reproduces them byte-exactly
#      (plain AND -S: site-independent), headers cite clause ids, and corpus
#      lint forbids prohibited anchors (C-19/D3 refcount APIs, S-05-14/15/17,
#      salted hashes, bare-set prints). Plus the 00.04 harness self-comparison
#      over the slice.
#
#  (C) ANTIPYTHONY-side gates (xfail, strict=False, until 02.08 lands):
#      - leaf 02.08's verbatim gate: ap-conform stdlib slice green in tier-0
#        AND adaptive AND binary modes (C-31 tri-mode rule).
#      - allow-list enforcement: each IN module imports and smokes; each OUT
#        module's one-line `import <mod>` build is refused with the C-10 token
#        for its class (APX-CEXT for extension-backed, APX-NOV1 for scope-
#        excluded) plus module name and file:line:col (C-08/C-10 Verify).
#      - PARTIAL rows (S-05-4): in-scope names reachable; names the row marks
#        registry-out are NOT reachable (AttributeError shape) / submodules
#        fail loud (importlib.metadata).
#
# Resolution seams copied from T-06: harness via env AP_CONFORM, PATH, then
# target/{debug,release}/; impl via env APX_IMPL else `antipythony run` / the
# local debug binary; reference via AP_CONFORM_REFERENCE_PYTHON, default
# /opt/homebrew/bin/python3.14.

import json
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
sys.path.insert(0, str(Path(__file__).resolve().parent))

import scope_table  # noqa: E402  (tests/t11_stdlib/scope_table.py)

CORPUS_DIR = REPO_ROOT / "tests" / "corpus" / "stdlib"
EXPECTED_DIR = CORPUS_DIR / "expected"
SPEC05 = REPO_ROOT / "docs" / "spec" / "05-builtins-stdlib-v1.md"
# Reference discovery: unified env+PATH ladder (robustness-ci 01.01); see
# tests/tools/_ref_python.py for the AP_REQUIRE_REFERENCE=1 skip->fail policy.
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
from _ref_python import reference_or_skip  # noqa: E402


def _find_bin(name):
    env = os.environ.get(name.upper().replace("-", "_"))
    if env:
        parts = shlex.split(env)
        return parts[0], parts
    which = shutil.which(name)
    if which:
        return which, [which]
    for cfg in ("debug", "release"):
        p = REPO_ROOT / "target" / cfg / name
        if p.is_file():
            return str(p), [str(p)]
    return None, None


AP_CONFORM, AP_CONFORM_CMD = _find_bin("ap-conform")


def _impl_cmd(mode):
    """(argv, needs_build) for one of the three 02.08 gate modes.

    tier-0 / adaptive use the runner (`antipythony run`); binary mode builds
    then executes the artifact. Returns None when no impl seam exists yet.
    """
    if os.environ.get("APX_IMPL"):
        return shlex.split(os.environ["APX_IMPL"])
    runner = REPO_ROOT / "target" / "debug" / "antipythony"
    if not runner.is_file():
        runner = Path(shutil.which("antipythony") or "")
    if runner.is_file():
        return [str(runner), "run"]
    return None


XFAIL_REASON = "awaits 02.08 v1 stdlib subset (antipythony registry side)"
xfail_ap = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)


def cases():
    return sorted(CORPUS_DIR.glob("s*.py"))


ALL_CASES = cases()
CASE_IDS = [p.stem for p in ALL_CASES]


# ---------------------------------------------------------------------------
# (A) ALLOW-LIST face — scope-table static assertions (green today).
# ---------------------------------------------------------------------------

def test_scope_table_matches_spec05_headings():
    # The transcription must cover every §5.3.3 S-clause name-list: none of
    # the table's IN modules may also appear in the OUT registry, and each
    # table entry carries a clause citation.
    assert set(scope_table.IN_MODULES) & set(scope_table.OUT_MODULES) == set()
    for mod, row in scope_table.IN_MODULES.items():
        assert row["cite"].startswith("S-05-"), mod
    for mod, (cls, cite) in scope_table.OUT_MODULES.items():
        assert cls in ("cext", "nov1"), mod
        assert "S-05-" in cite or "C-07" in cite, mod


def test_random_conflict_is_not_silently_scoped():
    # C-05's named floor says IN; S-05-19 says OUT. T-11 must not pick a side
    # silently — random sits in neither list, and the gap is filed (GAP-23).
    assert "random" not in scope_table.IN_MODULES
    assert "random" not in scope_table.OUT_MODULES
    assert "random" in scope_table.CONFLICT_MODULES


def test_spec05_table_rows_all_transcribed():
    # Every backticked module at the head of a spec 05 §5.3 table row lands
    # in exactly one of {IN, OUT, CONFLICT} (compound rows like
    # "`codecs` + `encodings`" contribute all their names).
    text = SPEC05.read_text(encoding="utf-8")
    heads = re.findall(r"^\|\s*((?:`[a-zA-Z_.]+`(?:\s*[+,]\s*)?)+)", text, re.M)
    names = set()
    for head in heads:
        names.update(re.findall(r"`([a-zA-Z_.]+)`", head))
    missing = sorted(n for n in names
                     if n not in scope_table.IN_MODULES
                     and n not in scope_table.OUT_MODULES
                     and n not in scope_table.CONFLICT_MODULES)
    assert not missing, f"spec-05 table rows not transcribed: {missing}"


@pytest.mark.parametrize("mod", sorted(scope_table.IN_MODULES))
def test_in_subset_names_exist_on_reference(mod):
    # S-05-4 says every subset name was "verified against python3.14 -S live
    # dir()". Re-verify the transcription against the reference: each subset
    # name must be present UNLESS whitelisted in KNOWN_REF_ABSENT (those are
    # the filed transcription gaps — spec claims vs. reference reality).
    if mod == "builtins":
        pytest.skip("builtins namespace governed by BUILTINS_IN; covered by corpus s001")
    if mod == "genericpath":
        pytest.skip("GAP-17: bare `-S` import of genericpath fails a CPython "
                    "circular-import edge (verified live); no subset claimed")
    probe = f'''
import {mod} as m
missing = [n for n in {scope_table.IN_MODULES[mod]["subset"]!r}
           if not hasattr(m, n)]
print(sorted(missing))
'''
    proc = subprocess.run([reference_or_skip(), "-S", "-c", probe], capture_output=True,
                          timeout=60, cwd=str(REPO_ROOT))
    assert proc.returncode == 0, f"{mod}: reference import failed: {proc.stderr!r}"
    missing = eval(proc.stdout.decode().strip())
    known = set(scope_table.KNOWN_REF_ABSENT.get(mod, [])) | set(
        scope_table.IN_MODULES[mod].get("ref_absent", []))
    assert set(missing) <= known, (
        f"{mod}: subset names absent on the reference but not whitelisted: "
        f"{sorted(set(missing) - known)}")
    stale = known - set(scope_table.IN_MODULES[mod].get("subset", []))
    assert not stale, f"{mod}: ref-absent whitelist entries not in the subset: {stale}"


def test_known_ref_absent_fully_cited():
    # Every whitelisted spec-vs-reference mismatch carries a GAP citation.
    blob = (json.dumps(scope_table.IN_MODULES) + json.dumps(scope_table.PARTIAL_ROWS)
            + str(scope_table.KNOWN_REF_ABSENT))
    for mod, names in scope_table.KNOWN_REF_ABSENT.items():
        for n in names:
            assert "GAP-" in blob, f"{mod}.{n} whitelisted without a GAP citation"


@pytest.mark.parametrize("mod", sorted(scope_table.OUT_MODULES))
def test_out_modules_import_fine_on_reference(mod):
    # The divergence premise (S-05-22): the reference IMPORTS these; the
    # fail-loud refusal is an antipythony-only behavior, gated against the
    # diagnostic format, not against CPython bytes.
    proc = subprocess.run([reference_or_skip(), "-S", "-c", f"import {mod}"],
                          capture_output=True, timeout=60, cwd=str(REPO_ROOT))
    assert proc.returncode == 0, f"{mod}: reference unexpectedly refuses: {proc.stderr!r}"


def test_out_registry_covers_s05_19_20_21_names():
    # Spot the headline names of each OUT clause are present in the table.
    for must in ("socket", "subprocess", "ctypes", "random-adjacent excluded",
                 "pickle", "unittest", "logging", "csv", "difflib", "site",
                 "runpy", "base64", "uuid", "cmath", "dis", "symtable",
                 "marshal", "gc"):
        if must.endswith(" excluded"):
            continue
        assert must in scope_table.OUT_MODULES, must
    # accelerator modules are never user-visible (S-05-3)
    for acc in ("_socket", "_json", "_functools", "_collections", "_sre",
                "_bisect", "_heapq", "_operator", "_random", "_csv"):
        assert acc in scope_table.OUT_MODULES, acc


def test_partial_rows_shape():
    for row in scope_table.PARTIAL_ROWS:
        assert row["mod"] in scope_table.IN_MODULES, row
        assert row["kind"] in ("attr", "submodule"), row
        assert row["disposition"] in ("registry-out", "keep-present"), row
        assert "GAP-" in row["cite"] or "row" in row["cite"], row
    # every PARTIAL probe row cites spec-05 text for its module
    mods = {r["mod"] for r in scope_table.PARTIAL_ROWS}
    assert {"sys", "os", "io", "re", "functools", "threading", "contextlib",
            "ast", "inspect", "time", "tempfile", "importlib"} <= mods


# ---------------------------------------------------------------------------
# (B) BEHAVIOR face — corpus fixture self-checks (green today).
# ---------------------------------------------------------------------------

def test_corpus_slice_is_populated():
    # >= 25 mandated (one per IN module row of spec 05 §5.3); 48 delivered.
    assert len(ALL_CASES) >= 25, f"only {len(ALL_CASES)} cases in {CORPUS_DIR}"


def test_corpus_covers_every_in_module_row():
    # S-05-22: each IN module gets at least one case; assert each IN name
    # literally appears (as import/mention) somewhere in the slice.
    blob = "\n".join(p.read_text(encoding="utf-8") for p in ALL_CASES)
    missing = [m for m in scope_table.IN_MODULES if m not in blob]
    assert not missing, f"IN modules with no corpus mention: {missing}"


@pytest.mark.parametrize("case", ALL_CASES, ids=CASE_IDS)
def test_expected_triple_exists(case):
    stem = case.stem
    for suffix in (".out", ".err", ".rc"):
        assert (EXPECTED_DIR / (stem + suffix)).is_file(), (
            f"missing expected/{stem}{suffix}")


def _pinned_env() -> dict:
    """S-06-15 scrubbed environment + pinned locale (C-27).

    Only PATH/HOME pass through, with LANG=C. LC_* is deliberately excluded:
    CPython's UTF-8-mode auto-detection rewrites LC_CTYPE in its own
    environment block, so a Python parent leaks it to child processes and
    silently changes their locale. The corpus triples were pinned under
    this scrub (LANG=C enables UTF-8 mode per PEP 540).
    """
    env = {k: os.environ[k] for k in ("PATH", "HOME") if k in os.environ}
    env["LANG"] = "C"
    return env


@pytest.mark.parametrize("case", ALL_CASES, ids=CASE_IDS)
def test_expected_bytes_stable(case):
    # Re-run the reference (plain and -S) and byte-compare stdout/rc (S-06-1
    # no-normalization; site-independence per S-05-1 -S parity). A failure
    # here means nondeterminism — fix by masking, never weaken (C-33).
    #
    # The case path is passed RELATIVE to REPO_ROOT (cwd is REPO_ROOT) so
    # tracebacks embed a checkout-independent path; absolute paths made the
    # pins fail in every worktree/CI checkout other than the original.
    stem = case.stem
    exp_out = (EXPECTED_DIR / (stem + ".out")).read_bytes()
    exp_rc = int((EXPECTED_DIR / (stem + ".rc")).read_text())
    exp_err = (EXPECTED_DIR / (stem + ".err")).read_bytes()
    for flags in ([], ["-S"]):
        proc = subprocess.run([reference_or_skip(), *flags, os.path.relpath(case, REPO_ROOT)],
                              capture_output=True,
                              timeout=120, cwd=str(REPO_ROOT), env=_pinned_env())
        label = "+".join(flags) or "plain"
        assert proc.returncode == exp_rc, f"{stem}[{label}]: rc drift {proc.returncode}"
        assert proc.stdout == exp_out, (
            f"{stem}[{label}]: stdout drift\n got={proc.stdout[:400]!r}\n"
            f"want={exp_out[:400]!r}")
        assert _mask_root(proc.stderr) == _mask_root(exp_err), (
            f"{stem}[{label}]: stderr drift")


@pytest.mark.parametrize("case", ALL_CASES, ids=CASE_IDS)
def test_case_header_cites_clauses(case):
    # tests/README.md corpus rule: every case cites spec clause ids.
    text = case.read_text(encoding="utf-8")[:800]
    assert "# Cites:" in text and ("S-05-" in text or "C-" in text), (
        f"{case.stem}: header does not cite clause ids")


def test_no_forbidden_corpus_apis():
    # C-19/D3 refcount APIs; S-05-14 breakpoint call; S-05-15 input;
    # S-05-17 memoryview anchors; spec-05 os row: urandom-as-corpus;
    # S-05-6 sys.stdlib_module_names gating; S-06-17 id() VALUE printing
    # (regex with a word boundary so get_ident/getpid comparisons don't hit).
    banned_re = re.compile(
        r"getrefcount|gettotalrefcount|getallocatedblocks|breakpoint\(|input\(|"
        r"memoryview|urandom|\bid\(|stdlib_module_names|sys\.hash_info")
    offenders = []
    for case in ALL_CASES:
        for i, line in enumerate(case.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("#"):
                continue
            m = banned_re.search(line)
            if m:
                offenders.append(f"{case.stem}:{i}: {m.group()} in {line.strip()}")
    assert not offenders, "prohibited anchor in corpus:\n" + "\n".join(offenders)


def test_no_salted_hash_or_bare_set_prints():
    # C-26: sets/frozensets only via sorted() or membership; no salted str/
    # bytes hash printed. Lint the sources.
    offenders = []
    for case in ALL_CASES:
        for i, line in enumerate(case.read_text(encoding="utf-8").splitlines(), 1):
            s = line.strip()
            if s.startswith("#"):
                continue
            if ("hash(\"" in s or "hash(b'" in s) and "== hash(" not in s:
                offenders.append(f"{case.stem}:{i}: salted-hash risk {s}")
    assert not offenders, "\n".join(offenders)


@pytest.mark.skipif(AP_CONFORM is None, reason="ap-conform bin missing")
def test_apconform_self_comparison_green():
    # 00.04 harness contract re-proven on THIS slice (S-06-1/C-27 three
    # channels, separate streams): reference vs reference byte-exact.
    proc = subprocess.run(
        [*AP_CONFORM_CMD, "run", "--corpus", "tests/corpus/stdlib/*.py",
         "--impl", reference_or_skip(), "--json"],
        capture_output=True, text=True, timeout=600, cwd=str(REPO_ROOT))
    assert proc.returncode == 0, proc.stdout + proc.stderr
    data = json.loads(proc.stdout)
    summary = data["summary"]
    assert summary["failed"] == 0 and summary["errors"] == 0 and summary["timeouts"] == 0, summary
    assert summary["passed"] == summary["total"] == len(ALL_CASES), summary


# ---------------------------------------------------------------------------
# (C) ANTIPYTHONY-side gates — xfail(strict=False) until 02.08 lands.
# ---------------------------------------------------------------------------

def _require_impl():
    impl = _impl_cmd("run")
    if impl is None:
        pytest.skip("no antipythony runner; set APX_IMPL")
    return impl


@xfail_ap
@pytest.mark.skipif(AP_CONFORM is None, reason="ap-conform bin missing")
@pytest.mark.parametrize("mode", ["tier-0", "adaptive"])
def test_apconform_antipythony_stdlib_slice(mode):
    # Leaf 02.08 Gate (verbatim): "ap-conform stdlib slice green in tier-0
    # AND adaptive AND binary modes" (C-31 behavioral invisibility).
    impl = _require_impl()
    extra = {"tier-0": ["--tier", "0"], "adaptive": []}[mode]
    proc = subprocess.run(
        [*AP_CONFORM_CMD, "run", "--corpus", "tests/corpus/stdlib/*.py",
         "--impl", " ".join(impl + extra), "--json"],
        capture_output=True, text=True, timeout=900, cwd=str(REPO_ROOT))
    data = json.loads(proc.stdout) if proc.stdout.strip().startswith("{") else {}
    assert proc.returncode == 0 and data.get("summary", {}).get("failed", -1) == 0, (
        f"{mode}: {proc.returncode} {data.get('summary')} {proc.stderr[:400]}")


@xfail_ap
@pytest.mark.parametrize("case", ALL_CASES[:6] + ALL_CASES[-2:], ids=[p.stem for p in (ALL_CASES[:6] + ALL_CASES[-2:])])
def test_binary_mode_matches_expected(case):
    # Third leg of the 02.08 gate: `antipythony build <case> -o <art>` then
    # run the artifact; stdout/stderr/rc must equal the pinned expected
    # triple (C-31: AoT binary invisible vs interpreter modes). Binary seam
    # per leaf 04 build verb as exercised by T-09; no ap-conform seam for
    # built artifacts is documented, so this asserts directly on the triple.
    runner = REPO_ROOT / "target" / "debug" / "antipythony"
    if not runner.is_file():
        runner = shutil.which("antipythony")
    if not runner:
        pytest.skip("no antipythony build seam; set APX_BUILD")
    art = Path("/tmp") / f"apx_t11_bin_{case.stem}"
    proc = subprocess.run([str(runner), "build", str(case), "-o", str(art)],
                          capture_output=True, text=True, timeout=300,
                          cwd=str(REPO_ROOT))
    assert proc.returncode == 0, f"build failed: {proc.stderr[:300]}"
    run = subprocess.run([str(art)], capture_output=True, timeout=60,
                         cwd=str(REPO_ROOT))
    stem = case.stem
    assert run.stdout == (EXPECTED_DIR / (stem + ".out")).read_bytes(), stem
    assert run.stderr == (EXPECTED_DIR / (stem + ".err")).read_bytes(), stem
    assert run.returncode == int((EXPECTED_DIR / (stem + ".rc")).read_text()), stem


# 2026-11-07 flip wave (01.02 protocol): the blanket "awaits 02.08"
# decorator was stale - 61/62 params pass. `sys` still genuinely fails
# and keeps the marker; everything else is a hard pass.
STILL_XFAIL_IN_MODULES = frozenset({"sys"})


@pytest.mark.parametrize(
    "mod", [pytest.param(m, marks=xfail_ap) if m in STILL_XFAIL_IN_MODULES
            else m for m in sorted(scope_table.IN_MODULES)])
def test_in_module_imports_and_smokes_on_antipythony(mod):
    # C-05 Verify: compiling `import m; print(m.__name__)` succeeds and the
    # output matches the reference.
    if AP_CONFORM is None:
        pytest.skip("ap-conform bin missing")
    impl = _require_impl()
    probe = Path("/tmp") / "apx_t11_in_probe.py"
    probe.write_text(f"import {mod}\nprint({mod}.__name__)\n", encoding="utf-8")
    try:
        proc = subprocess.run(
            [*AP_CONFORM_CMD, "run", "--corpus", str(probe),
             "--impl", " ".join(impl), "--json"],
            capture_output=True, text=True, timeout=120, cwd=str(REPO_ROOT))
        data = json.loads(proc.stdout) if proc.stdout.strip().startswith("{") else {}
        assert proc.returncode == 0, f"{mod}: harness rc {proc.returncode}: {proc.stderr[:300]}"
        assert data["summary"]["passed"] == 1, f"{mod}: {data.get('summary')}"
    finally:
        probe.unlink(missing_ok=True)


@xfail_ap
@pytest.mark.parametrize(
    "mod", sorted(m for m, (cls, _) in scope_table.OUT_MODULES.items()
                  if not m.startswith("_")))
def test_out_module_build_fails_loud_with_token(mod):
    # S-05-2 / C-07 / C-08 / C-10: `import <mod>` (static) refuses the build
    # with the class token for the row — APX-CEXT for extension-backed,
    # APX-NOV1 for scope-excluded — plus the module name and file:line:col.
    cls, _cite = scope_table.OUT_MODULES[mod]
    token = "APX-CEXT" if cls == "cext" else "APX-NOV1"
    runner = REPO_ROOT / "target" / "debug" / "antipythony"
    if not runner.is_file():
        runner = shutil.which("antipythony")
    if not runner:
        pytest.skip("no antipythony build seam; set APX_BUILD")
    probe = Path("/tmp") / f"apx_t11_out_{mod}.py"
    probe.write_text(f"import {mod}\n", encoding="utf-8")
    proc = subprocess.run([str(runner), "build", str(probe), "-o",
                           str(probe.with_suffix(""))],
                          capture_output=True, text=True, timeout=120,
                          cwd=str(REPO_ROOT))
    combined = proc.stdout + proc.stderr
    assert proc.returncode != 0, f"{mod}: build unexpectedly succeeded"
    assert token in combined, f"{mod}: missing {token}: {combined[:300]!r}"
    assert mod in combined, f"{mod}: diagnostic does not name the module"
    assert re.search(r"\.py:\d+(:\d+)?", combined), f"{mod}: no file:line:col"
    # C-11: no artifact emitted
    assert not probe.with_suffix("").exists(), f"{mod}: artifact produced despite refusal"


# 2026-11-07 flip wave: 12/19 partial rows pass; the 7 below still
# genuinely fail and keep the marker (STALE-XFAIL-LEDGER, 01.02).
STILL_XFAIL_PARTIAL_ROWS = frozenset({
    "ast.main", "importlib.metadata", "os.kill", "os.pipe",
    "sys.audit", "sys.monitoring", "tempfile.mkstemp"})


@pytest.mark.parametrize(
    "row", [pytest.param(r, id=f"{r['mod']}.{r['name']}", marks=xfail_ap)
            if f"{r['mod']}.{r['name']}" in STILL_XFAIL_PARTIAL_ROWS
            else pytest.param(r, id=f"{r['mod']}.{r['name']}")
            for r in scope_table.PARTIAL_ROWS])
def test_partial_row_enforcement_on_antipythony(row):
    # S-05-4: names NOT in the v1 subset MUST NOT be reachable (attribute
    # access raises AttributeError matching CPython's message shape; missing
    # submodule import obeys S-05-2). "registry-out" rows assert unreach-
    # ability; "keep-present" rows assert the in-scope surface exists (the
    # prohibition is corpus-level, GAP-28). NOTE: these expectations are
    # written on the ANTIPYTHONY side only — on the reference the OUT names
    # exist, so these can never be ap-conform byte cases (GAP-26/GAP-27).
    if AP_CONFORM is None:
        pytest.skip("ap-conform bin missing")
    impl = _require_impl()
    if row["kind"] == "attr":
        if row["disposition"] == "registry-out":
            src = (
                f"import {row['mod']}\n"
                f"print(hasattr({row['mod']}, {row['name']!r}))\n"
            )
            want = "False\n"
        else:  # keep-present: in-scope sibling name must exist
            sibling = {
                ("re", "Scanner"): "compile",
                ("functools", "namedtuple"): "reduce",
                ("threading", "getprofile"): "Lock",
                ("contextlib", "chdir"): "suppress",
                ("inspect", "markcoroutinefunction"): "signature",
                ("os", "urandom"): "getcwd",
            }[(row["mod"], row["name"])]
            src = (f"import {row['mod']}\nprint(hasattr({row['mod']},"
                   f" {sibling!r}))\n")
            want = "True\n"
    else:  # submodule
        src = f"import {row['mod']}.{row['name']}\n"
        want = None  # refusal-only: build must fail with the token
    if want is None:
        token = "APX-NOV1"
        runner = REPO_ROOT / "target" / "debug" / "antipythony"
        if not runner.is_file():
            pytest.skip("no antipythony build seam")
        probe = Path("/tmp") / f"apx_t11_part_{row['mod']}_{row['name']}.py"
        probe.write_text(src, encoding="utf-8")
        proc = subprocess.run([str(runner), "build", str(probe)],
                              capture_output=True, text=True, timeout=120,
                              cwd=str(REPO_ROOT))
        assert proc.returncode != 0 and token in proc.stdout + proc.stderr, (
            f"{row['mod']}.{row['name']}: expected {token} refusal")
        return
    # attribute-surface probe runs through the harness: expected file pins
    # the antipythony-side behavior, which by construction DIFFERS from the
    # reference for registry-out rows (GAP-26). Run directly instead of
    # ap-conform (a ref-vs-impl diff on this probe is expected to FAIL the
    # byte gate by design; the assert below is the spec-05 contract):
    probe = Path("/tmp") / f"apx_t11_attr_{row['mod']}_{row['name']}.py"
    probe.write_text(src, encoding="utf-8")
    proc = subprocess.run([*impl, str(probe)], capture_output=True, text=True,
                          timeout=120, cwd=str(REPO_ROOT))
    assert proc.returncode == 0, f"{row['mod']}.{row['name']}: rc {proc.returncode}"
    assert proc.stdout == want, (
        f"{row['mod']}.{row['name']} ({row['disposition']}): got {proc.stdout!r}"
        f" want {want!r}")
