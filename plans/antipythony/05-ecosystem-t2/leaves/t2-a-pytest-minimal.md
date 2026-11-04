# Leaf T2-a: pytest under antipythony (ecosystem tier 2)

## Prereq state (post-T1)
scrapy 2.19 imports + runs (docs/t1-gate.md). unittest gate landed in Plan 02.
pytest requires: unittest, traceback (deep frames), assertion rewriting (AST),
pluggy hook machinery, pathlib, importlib.metadata, warnings, argparse.

## Target (master.md tier table: pytest T2)
Minimal gate: `pytest --version` + run a trivial test file (1 pass/1 fail/1 skip)
with correct terminal output shape (dots, short summary lines) byte-comparable
vs CPython-run pytest. Full suite runs are stretch.

## First probes (run before coding)
1. uv pip install pytest into the staged site-packages (pure wheel, pluggy +
   iniconfig + packaging deps)
2. `antipythony run -m pytest --version`
3. `antipythony run -m pytest test_trivial.py -v` -> fix import chain blockers
   iteratively (same protocol as T1)

## Expected hard spots
- assertion rewriting compiles AST via py module -> our ast surface must cover
  pytest's rewrite visitor (Name/Compare/Assert transforms)
- pluggy's _Result/multicall uses inspect.signature + closures heavily
- pytest's own py.typed/entrypoint loading (importlib.metadata entry points)

## Gates
- G-T2a: trivial test file runs with byte-exact terminal summary vs CPython
- all existing gates hold (cargo 0 failed, pytest 1912+ on CPython side,
  corpus non-decreasing)
