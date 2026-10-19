# T-02 — IR shape test suite (gates leaf 01.02).
#
# Authorship: written from the spec pack ONLY (docs/spec/01, docs/spec/02,
# docs/compat-contract.md, plans/antipythany/master.md) — no implementation source
# read. See plans/antipythany/tests/T-02-ir-shape.md for per-test citations.
#
# Contract under test: bin `ap-ir-dump <file>` parses the Python file and prints an
# indented IR dump, one node per line, `node-kind` + optional ` detail`.
# Leaf 01.02 has landed; the bin exists and all tests are expected to pass
# (the xfail envelope was removed 2026-10-19, stale-xfail ledger).

import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
SUITES = REPO_ROOT / "tests" / "t02_ir_shape" / "fixtures"

# Each entry: (test-stem, source lines, expected regex line kinds).
# "exactly 1" = the dump contains exactly one line starting with that kind.
CONSTRUCT_CASES = [
    # spec 01 §1: binary arithmetic (S-01-1); plan-01 Expr list `binop`
    ("binop_single", ["x = a + b"], "BinOp"),
    # spec 01 S-01-34: ordering comparison
    ("compare_single", ["b = a < c"], "Compare"),
    # spec 01 S-01-34: chained comparison (structure unpinned -> count >= 1)
    ("compare_chain", ["b = a < c < d"], "Compare"),
    # spec 02 S-02-9: call with positional and keyword args
    ("call_kwargs", ["f(a, k=1)"], "Call"),
    # spec 02 S-02-11 / S-02-38: lambda is a first-class function expression
    ("lambda", ["g = lambda x: x"], "Lambda"),
    # spec 02 S-02-5: comprehension has its own implicit scope
    ("comprehension", ["ys = [i for i in xs]"], "Comprehension"),
    # spec 02 S-02-20/21: generator function, yield suspends
    ("yield", ["def g():", "    yield 1"], "Yield"),
    # plan-01 Expr list names `await` (tier T1 scrapy requires async shapes)
    ("await", ["async def f():", "    await g()"], "Await"),
    # spec 01 S-01-16: f-string interpolation (kind name unpinned -> G-2, loose)
    ("fstring", ["s = f\"a{x}b\""], None),
    # spec 02 S-02-15
    ("if", ["if a:", "    pass"], "If"),
    # spec 02 S-02-16
    ("while", ["while a:", "    pass"], "While"),
    # spec 02 S-02-17
    ("for", ["for i in xs:", "    pass"], "For"),
    # spec 02 S-02-30/32/33
    (
        "try_except_finally",
        [
            "try:",
            "    a = 1",
            "except ValueError:",
            "    pass",
            "finally:",
            "    pass",
        ],
        "Try",
    ),
    # plan-01 Stmt list names `match`
    ("match", ["match x:", "    case 1:", "        pass"], "Match"),
    # spec 02 S-02-40/41
    ("with", ["with cm:", "    pass"], "With"),
    # spec 02 S-02-5 (class bodies); spec 03 preview — shape only
    ("class_def", ["class C:", "    pass"], "ClassDef"),
    # spec 02 S-02-7/8: defaults at def time; positional/kwonly parameter kinds
    (
        "function_def",
        ["def f(a, b=2, *, c):", "    pass"],
        "FunctionDef",
    ),
]


def _ap_ir_dump_cmd() -> list[str]:
    """Command line for the contract bin (override with AP_IR_DUMP env)."""
    override = os.environ.get("AP_IR_DUMP")
    if override:
        import shlex

        return shlex.split(override)
    if shutil.which("ap-ir-dump"):
        return ["ap-ir-dump"]
    # Default: cargo from the repo root. Bin does not exist yet until 01.02.
    return ["cargo", "run", "--quiet", "--bin", "ap-ir-dump"]


def run_dump(source: str) -> subprocess.CompletedProcess:
    path = SUITES / "t02_tmp_case.py"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(source)
    return subprocess.run(
        _ap_ir_dump_cmd() + [str(path)],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=120,
    )


def dump_lines(source: str) -> list[str]:
    proc = run_dump(source)
    assert proc.returncode == 0, (
        f"ap-ir-dump exited {proc.returncode}\nstderr:\n{proc.stderr}"
    )
    return [ln for ln in proc.stdout.splitlines() if ln.strip()]


def lines_matching(lines: list[str], kind: str) -> list[str]:
    return [ln for ln in lines if ln.lstrip().startswith(kind)]


@pytest.mark.parametrize(
    "name,src,kind",
    CONSTRUCT_CASES,
    ids=[c[0] for c in CONSTRUCT_CASES],
)
def test_construct_single_node(name, src, kind):
    # G-2: f-string kind vocabulary unpinned until the spec addendum; assert only
    # that some non-literal node line appears beyond the literal parts.
    if kind is None:
        lines = dump_lines("\n".join(src) + "\n")
        assert any(not ln.lstrip().startswith(("Constant", "Literal")) for ln in lines), (
            "f-string dump shows no node beyond literal parts"
        )
        return
    lines = dump_lines("\n".join(src) + "\n")
    assert len(lines_matching(lines, kind)) == 1, (
        f"expected exactly one {kind} line in dump:\n" + "\n".join(lines)
    )


def test_binop_chain_count():
    # `x + y + z` is left-associative: two BinOp nodes. Associativity is pinned
    # observable semantics (S-01-2: `**` right-assoc, `3 ** 3 ** 2` == 19683), so
    # the IR must keep the operator tree shape the grammar produces.
    lines = dump_lines("x = a + b + c\n")
    assert len(lines_matching(lines, "BinOp")) == 2, (
        "left-assoc `a + b + c` must yield two BinOp nodes:\n" + "\n".join(lines)
    )


@pytest.mark.parametrize("src", [["x = a + b"], ["x = a + b + c"]], ids=["one", "chain"])
def test_dump_deterministic(src):
    # master plan 01: "stability matters more than completeness"; 01.02 exposes
    # "stable PartialEq on all nodes (needed for golden tests)"; C-10 principle:
    # machine-readable output tests rely on MUST NOT vary between runs.
    first = "\n".join(dump_lines("\n".join(src) + "\n"))
    second = "\n".join(dump_lines("\n".join(src) + "\n"))
    assert first == second and first != ""


def test_all_constructs_no_crash():
    # plan-01 01.02 gate: "builder constructs all corpus constructs". One program
    # touching every Expr/Stmt kind spec 01+02 require; parse+dump must exit 0.
    program = "\n".join(
        [
            "x = a + b + c",
            "d = a < b < c",
            "f(a, k=1)",
            "g = lambda p: p",
            "ys = [i for i in xs]",
            "",
            "def gen():",
            "    yield 1",
            "",
            "async def af():",
            "    await g()",
            "",
            "s = f\"a{x}b\"",
            "",
            "if a:",
            "    pass",
            "while a:",
            "    pass",
            "for i in xs:",
            "    pass",
            "",
            "try:",
            "    a = 1",
            "except ValueError:",
            "    pass",
            "finally:",
            "    pass",
            "",
            "match x:",
            "    case 1:",
            "        pass",
            "",
            "with cm:",
            "    pass",
            "",
            "class C:",
            "    pass",
            "",
            "def fn(q, r=2, *, t):",
            "    pass",
        ]
    )
    lines = dump_lines(program + "\n")
    assert len(lines) > 10
