# 02.04a - Import-of-File Machinery (design)

## Problem
Third-party pure-Python packages (twisted, scrapy, pytest) are directories of
.py files. Our v0 executor raises ImportError for anything not in the native
registry. Running targets REQUIRES file-based imports: sys.path resolution,
packages (__init__.py), namespace packages, relative imports.

## Design

### Module resolution order (per import statement)
1. sys.modules cache (module identity: same object re-imported)
2. native registry (02.08-lite hook - math, json, etc.)
3. File finder over sys.path: for each entry p in sys.path:
   a. p/<name>/__init__.py  -> package
   b. p/<name>.py           -> module
   (namespace packages: dir without __init__ - v1: support, mark __path__)
4. ImportError: module not found (CPython message format:
   No module named 'x'; relative: attempted relative import with no known
   parent package)

### Execution model
- Module file = compile (parse->lower->verify) + execute in a FRESH module
  namespace whose __name__/__file__/__package__ are set; the resulting
  namespace dict IS the module object (v0: module = dict + metadata wrapper).
- sys.modules[name] = namespace BEFORE executing body (CPython circular-
  import semantics: partial module visible).
- Packages: __path__ = [pkg_dir]; submodule resolution descends.
- Relative imports: from . import x / from .mod import y - resolved against
  __package__ per CPython rules (cite S-04-*).

### sys.path bootstrap
- [script_dir] + vendored_stdlib_dir + ANTPYTHANY_PATH env + site-packages
  dir when Plan 04 wheel-fetch lands. ap-conform/ap-run-eval set
  script_dir = the corpus case's directory.

### Compile cache
- Same content-addressed cache as Plan 04 leaf 04.02 (key: source hash +
  IR version): .apc files next to source or in ~/.cache/antipythany/.
  v0 (this leaf): no cache, compile every import - measure first.

### Integration with ap-conform
- sys.path for corpus runs: [case_dir, vendored_stdlib] so tests exercise
  file imports against the vendored tree.

## Gates
- import of a vendored pure module: import pathlib; p = pathlib.Path('.')-> works
- package: import unittest; unittest.TestCase resolvable (class machinery)
- relative: a package with submodules importing from .
- circular: a imports b imports a (partial-module semantics)
- ImportError texts byte-exact (probe python3.14)
- stdlib corpus slice delta reported
