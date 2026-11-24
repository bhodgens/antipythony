"""F-STRUCT-1 minimal repro: re-entrant lazy import of a partially-initialized package.

Package P:
  P/__init__.py  — PEP-562 __getattr__ that lazy-imports P.main
  P/main.py      — `from collections.abc import Callable` (typing chain) AND
                   `from P import helper` — a from-import of P WHILE P is
                   partially initialized.

CPython: the from-import of `P.helper` inside main.py resolves against the
sys.modules entry for P (partially initialized) — module dict first, then
module __getattr__ — WITHOUT re-entering the importer. So `helper` resolves
via P.__getattr__ -> import_module('.sub', 'P') -> P.sub.OUTPUT and prints it.

A runtime that re-enters the importer on the from-import instead of
consulting the (partially-initialized) module's __getattr__ raises
ImportError: cannot import name 'helper' from 'P'.
"""
import sys

import P

print("P-ATTR:", P.OUTPUT)
print("DONE")
