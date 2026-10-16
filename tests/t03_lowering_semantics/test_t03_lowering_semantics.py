# T-03 — lowering semantics test suite (gates leaf 01.03).
#
# Authorship: written from the spec pack ONLY (docs/spec/01, docs/spec/02,
# docs/spec/04, docs/compat-contract.md, plans/antipythany/master.md) — no
# implementation source read. See
# plans/antipythany/tests/T-03-lowering-semantics.md for the scope-classification
# derivation table, per-test citations, and CONTRACT-GAPS G-1..G-8.
#
# Contract under test (leaf 01.03 Expose + this leaf's seam extension):
#   ap_ir::lower(ast) -> Result<ir::Module, LowerError>, reachable via the
#   `ap-ir-dump` bin seam established by T-02, extended with:
#     `ap-ir-dump --scopes <file>`  -> one line per function-like scope:
#       SCOPE name=<frame-name> cell=[a,b] free=[c] global=[d,e] [flags=[...]]
#   Frame names follow S-02-38 (`<module>`, `<lambda>`, innermost co_name;
#   `<listcomp>` for comprehension scopes — comprehension-name decision in G-5).
#
# What is asserted:
#   1. Per-scope cell/free/global classification (spec-pinned by S-02-2/3/5/12/13)
#      against 20 checked-in golden serializations under goldens/.
#   2. Determinism: two dumps of the same program are byte-identical (01.03 gate:
#      "all 20 goldens byte-stable"; C-10 machine-stability principle).
#   3. Observable-effect order preserved (plan-01 design rule + S-02 preamble).
# Node shapes the spec does NOT pin are deliberately NOT asserted — see
# CONTRACT-GAPS in the leaf doc (G-3: invariants over exact trees).
#
# The bin/flag seam does not exist yet (leaf 01.03 not started); every test is
# xfail(strict=False), so the suite reports all-xfail and zero errors until the
# seam lands. Golden-vs-corpus self-consistency is asserted inside
# test_scope_classification_matches_golden BEFORE the seam call and was verified
# at authoring time by the CPython 3.14.7 reference run recorded in the goldens.

import json
import os
import re
import shlex
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SUITE = Path(__file__).resolve().parent
CORPUS = SUITE / "corpus"
GOLDENS = SUITE / "goldens"

XFAIL_REASON = "awaits 01.03 ap_ir::lower + ap-ir-dump --scopes seam"

PROGRAMS = sorted(p.stem for p in CORPUS.glob("*.py"))
assert len(PROGRAMS) == 20, "T-03 corpus must hold exactly the 20 leaf programs"


def _ap_ir_dump_cmd() -> list[str]:
    """Command line for the contract bin (override with AP_IR_DUMP env)."""
    override = os.environ.get("AP_IR_DUMP")
    if override:
        return shlex.split(override)
    if shutil.which("ap-ir-dump"):
        return ["ap-ir-dump"]
    # Default: cargo from the repo root. Bin does not exist yet until 01.02/01.03.
    return ["cargo", "run", "--quiet", "--bin", "ap-ir-dump"]


def run_dump(file: Path, extra=()) -> subprocess.CompletedProcess:
    return subprocess.run(
        _ap_ir_dump_cmd() + list(extra) + [str(file)],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=120,
    )


def scope_lines(proc: subprocess.CompletedProcess) -> list[str]:
    assert proc.returncode == 0, (
        f"ap-ir-dump --scopes exited {proc.returncode}\nstderr:\n{proc.stderr}"
    )
    lines = [ln for ln in proc.stdout.splitlines() if ln.strip().startswith("SCOPE")]
    assert lines, f"no SCOPE lines in output:\n{proc.stdout}"
    return lines


SCOPE_RE = re.compile(
    r"SCOPE\s+name=(?P<name>\S+)\s+"
    r"cell=\[(?P<cell>[^\]]*)\]\s+"
    r"free=\[(?P<free>[^\]]*)\]\s+"
    r"global=\[(?P<glob>[^\]]*)\]"
    r"(?:\s+flags=\[(?P<flags>[^\]]*)\])?"
)


def _names(s: str) -> frozenset:
    return frozenset(t.strip() for t in s.split(",") if t.strip()) if s.strip() else frozenset()


def parse_scope_line(line: str) -> dict:
    m = SCOPE_RE.search(line)
    assert m, f"line does not match SCOPE grammar: {line!r}"
    return {
        "name": m["name"],
        "cell": _names(m["cell"]),
        "free": _names(m["free"]),
        "global": _names(m["glob"]),
        "flags": _names(m["flags"] or ""),
    }


def scope_from_golden(entry: dict) -> dict:
    return {
        "name": entry["name"],
        "cell": frozenset(entry.get("cell", [])),
        "free": frozenset(entry.get("free", [])),
        "global": frozenset(entry.get("global", [])),
        "flags": frozenset(entry.get("flags", [])),
    }


def scope_key(sc: dict) -> tuple:
    """Multiset key. Names may repeat (two <listcomp> scopes) and ordering of
    dump lines is unpinned, so classification is compared as a multiset (G-5).
    flags are EXCLUDED from the strict key (field vocabulary unpinned, G-6) and
    checked as golden-subset separately."""
    return (sc["name"], sc["cell"], sc["free"], sc["global"])


def load_golden(program: str) -> dict:
    return json.loads((GOLDENS / f"{program}.scope.json").read_text())


def check_golden_self_consistency(program: str) -> dict:
    """Validate the golden against its corpus program BEFORE invoking the seam:
    citation manifest matches the corpus header (corpus rule, tests/README.md),
    reference stdout/rc present, scope rows non-empty. These checks pass today
    (authoring-time verified); the seam call below is what xfails until 01.03."""
    golden = load_golden(program)
    src = (CORPUS / f"{program}.py").read_text()
    cites = set(re.findall(r"# Cites: (S-\d+-\d+)", src))
    assert cites, f"corpus header for {program} cites no clause ids"
    assert set(golden["cites"]) == cites, (
        f"{program}: golden cites {sorted(golden['cites'])} but corpus cites {sorted(cites)}"
    )
    assert isinstance(golden["cpython_3147_stdout"], str), "golden missing reference stdout"
    assert golden["cpython_3147_rc"] in (0, 1), "golden missing reference rc"
    assert golden["scopes"], "golden has no scope entries"
    return golden


xfail_lower = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)


# --- 1. scope classification goldens ----------------------------------------


@xfail_lower
@pytest.mark.parametrize("program", PROGRAMS, ids=PROGRAMS)
def test_scope_classification_matches_golden(program):
    """cell/free/global per scope, hand-derived from S-02-1/2/3/5/12/13 (full
    derivation table in the leaf doc). Strict: multiset equality of
    (name, cell, free, global) tuples. Global-column convention (module-OR-
    builtin level refs) pinned in G-2."""
    golden = check_golden_self_consistency(program)
    lines = scope_lines(run_dump(CORPUS / f"{program}.py", ["--scopes"]))
    got = [parse_scope_line(ln) for ln in lines]
    want = [scope_from_golden(e) for e in golden["scopes"]]
    assert sorted(scope_key(s) for s in got) == sorted(scope_key(s) for s in want), (
        f"{program}: scope dump {sorted(map(scope_key, got))} != golden "
        f"{sorted(map(scope_key, want))}"
    )
    # flags: golden markers must appear (subset check — extra dump flags OK,
    # missing generator/yield-from markers fail; strictest reading, G-6).
    for w in want:
        if w["flags"]:
            matches = [g for g in got if scope_key(g) == scope_key(w)]
            assert matches, f"{program}: no dump scope for golden {w['name']}"
            assert any(w["flags"] <= g["flags"] for g in matches), (
                f"{program}: scope {w['name']} golden flags {sorted(w['flags'])} "
                f"absent from dump flags {[sorted(g['flags']) for g in matches]}"
            )


# --- 2. determinism -----------------------------------------------------------


@xfail_lower
@pytest.mark.parametrize("program", PROGRAMS, ids=PROGRAMS)
def test_dump_is_byte_deterministic(program):
    """01.03 gate 'all 20 goldens byte-stable' + C-10 machine-stability: the
    same input dumped twice must be byte-identical (both the plain IR dump and
    the --scopes view)."""
    f = CORPUS / f"{program}.py"
    p1, s1 = run_dump(f), run_dump(f, ["--scopes"])
    p2, s2 = run_dump(f), run_dump(f, ["--scopes"])
    for pr in (p1, s1, p2, s2):
        assert pr.returncode == 0, f"{program}: dump exited {pr.returncode}\n{pr.stderr}"
        assert pr.stdout.strip(), f"{program}: dump produced no output"
    a = p1.stdout + "\n@@\n" + s1.stdout
    b = p2.stdout + "\n@@\n" + s2.stdout
    assert a == b, f"{program}: dumps differ between runs"


# --- 3. no-crash totality + effect-order invariants ----------------------------


@xfail_lower
def test_all_corpus_programs_lower_without_crash():
    """plan-01 01.03 scope: the full v1 subset lowered without crashes — exit 0
    and non-empty output on all 20 programs, both dump views."""
    for program in PROGRAMS:
        for extra in ([], ["--scopes"]):
            proc = run_dump(CORPUS / f"{program}.py", extra)
            assert proc.returncode == 0, (
                f"{program} {' '.join(extra)}: exit {proc.returncode}\n{proc.stderr}"
            )
            assert proc.stdout.strip(), f"{program}: empty dump"


@xfail_lower
def test_lowering_preserves_observable_effect_order():
    """plan-01 design rule: 'lowering must not drop or reorder operations with
    observable effects' (spec-02 preamble: every construct defines the observable
    order tests may rely on). The dump is assumed an in-order traversal (an
    unpinned presentation choice, listed as G-3), so for a straight-line print
    sequence the first dump line mentioning each literal must appear in source
    order. Only RELATIVE line order is asserted — exact tree shape is unpinned."""
    src = SUITE / "t03_order_case_tmp.py"
    src.write_text('print("A")\nprint("B")\nprint("C")\n')
    try:
        proc = run_dump(src)
        assert proc.returncode == 0, f"order case failed to dump: {proc.stderr}"
        lines = proc.stdout.splitlines()
    finally:
        src.unlink(missing_ok=True)
    idx = {}
    for token in ("A", "B", "C"):
        hits = [i for i, ln in enumerate(lines) if re.search(rf"[\"']{token}[\"']", ln)]
        assert hits, f"literal {token} absent from dump:\n" + "\n".join(lines)
        idx[token] = hits[0]
    assert idx["A"] < idx["B"] < idx["C"], (
        f"effect order not preserved: {idx}\ndump:\n" + "\n".join(lines)
    )
