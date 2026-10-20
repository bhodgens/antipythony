# Branch 02 - Lint Gates: PROGRESS

## 02.02 IMPLEMENTED NOT COMMITTED (2026-10-19, leaf agent)

### Reference probes (live /opt/homebrew/bin/python3.14 = 3.14.7, `-S`, fresh process per shape)
Command shape: `python3.14 -S -c "__import__('<shape>')"` (stderr tail is the
exception line; exit 1 on uncaught). Pinned bytes:

    __import__('')        ValueError: Empty module name            <-- NOT "ImportError: invalid module name" (review paraphrase is wrong)
    __import__('.')       ModuleNotFoundError: No module named '.'
    __import__('..')      ModuleNotFoundError: No module named '.'
    __import__('...')     ModuleNotFoundError: No module named '.'
    __import__('..a')     ModuleNotFoundError: No module named '.'
    __import__('../evil') ModuleNotFoundError: No module named '.'
    __import__('.a')      ModuleNotFoundError: No module named '.a'
    __import__('.a.b')    ModuleNotFoundError: No module named '.a'
    __import__('.os')     ValueError: Empty module name   (quirk: os.py resolves, head-repartition is empty)
    __import__('.json')   ValueError: Empty module name
    __import__('.keyword')ValueError: Empty module name
    __import__('.os.path')ValueError: Empty module name
    __import__('.sys')    ModuleNotFoundError: No module named '.sys'  (sys.modules cache does NOT count as resolves)
    __import__('.os/')    ModuleNotFoundError: No module named '.os/'
    __import__('a..b')    ModuleNotFoundError: No module named 'a'
    __import__('a.b.')    ModuleNotFoundError: No module named 'a'
    __import__('os/')     ModuleNotFoundError: No module named 'os/'
    __import__('os/..')   ModuleNotFoundError: No module named 'os/'
    __import__('/etc/..') ModuleNotFoundError: No module named '/etc/'
    __import__('a/b/..')  ModuleNotFoundError: No module named 'a/b/'
    __import__('json/../../etc') ModuleNotFoundError: No module named 'json/'
    __import__('a b')     ModuleNotFoundError: No module named 'a b'
    name='a'+chr(92)+'b'  ModuleNotFoundError: No module named 'a\\b'      (repr-escaped backslash)
    name="a"+chr(39)+"q"  ModuleNotFoundError: No module named "a'q"        (repr switches to double quotes)
    __import__('os.path') (and 'os', 'json', 'sys', 'builtins', 'keyword') -> exit 0, valid imports unaffected
    traceback shape (uncaught): stderr LAST line is the exception line; the
    frames above it are reference-interpreter-specific and are NOT compared
    (precedent: import_differential.rs::import_error_texts_byte_exact).

### Guard placement (crates/ap-run/src/importer.rs)
- `segment_is_safe(seg)`: rejects '', '.', '..', and any segment containing
  '/' or '\\' — checked in BOTH `find_on_path` and `find_submodule` before
  any `Path::join` (finding #16 core: pre-guard, find_on_path("") joined the
  base dir and returned it as a namespace package; __import__('') "succeeded"
  with __path__=[sys.path base], '/etc/..' imported /etc, '../evil' escaped
  sys.path entries).
- `import_file_module` head: empty name -> `ValueError('Empty module name')`
  (CPython _sanity_check, byte-exact); leading-dot family ->
  ModuleNotFoundError with the probe-pinned reported prefix ('.', '.x'
  chain-truncated) or the '.x'-resolves Empty-module-name quirk; all with
  CPython-repr name embedding via new `py_name_repr` (backslash doubling +
  quote switching, probe-pinned).
- Call-site reach: `__import__` builtin (builtins.rs ~line 3035) already
  routes arbitrary str names into `importer::import_module` — no change
  outside importer.rs was needed (gap below is only about *which* names
  reach it, not whether it is reachable).

### Gates run
1. `cargo test -p ap-run --test import_guard_differential` -> 9 passed
   (also green under AP_REQUIRE_REFERENCE=1).
2. `cargo test -p ap-run --test import_differential` -> 6 passed (unchanged).
3. Full `cargo test -p ap-run` -> 156 passed, 0 failed, 1 pre-existing
   ignored.
4. Valid dotted imports: os.path / json / pkg.mod / from-imports all resolve
   (test valid_imports_still_resolve, byte-exact vs reference).
5. Corpus untouched: ap-conform `tests/corpus/**/*.py` = 270 cases,
   224 passed / 46 failed BOTH with and without this change (verified by
   reverting importer.rs to HEAD, rebuilding, re-running) — no regression;
   the 46 are sibling-plan WIP.
6. rustfmt --check clean on importer.rs + the new test; cargo clippy emits
   NO warnings from the added code (pre-existing warnings untouched).

### Deliberate scope cuts / gaps (honest)
- NO corpus cases added (per dispatch: tests/corpus lints + traceability
  need spec-cite review by a different agent; the Rust differential probes
  are the fence).
- `os.nope`-family ("...; 'os' is not a package") NOT implemented: ap's os
  and json are NATIVE modules (no __path__), while CPython's os.path/json
  children resolve through package __path__; implementing the suffix on the
  sys.modules-parent condition regressed 'json.decoder'/'os.path' (ap
  resolves them through the native-delegation arm). Forcing it is a
  Plan-02 native-module __path__ fidelity change (product semantics),
  which master.md forbids this campaign from making. Recorded here; the
  child segment is still join-guarded on that arm by find_on_path's own
  check.
- 'json.decoder' divergence (ref OK / ap ModuleNotFoundError) is PRE-EXISTING
  (native json lacks the decoder submodule surface) — not introduced here;
  corpus-failed cases likewise pre-exist.
- importlib.import_module(name) TypeError-package variants (e.g. "the
  'package' argument is required...") are NOT covered: ap's importlib
  surface routes through the same importer but the builtins-level
  import_module wrapper is a different, unpinned surface; __import__ is the
  pinned contract for this leaf.
- The '.x' quirk (ValueError when the segment after the leading dot
  resolves) approximates CPython's spec-vs-cache subtlety; probes '.sys' vs
  '.os' pin the two sides ap implements.
