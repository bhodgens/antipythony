# 02.08 (v1 stdlib subset) Coverage Audit — 2026-11-11

Leaf: audit-only. Tree: leaf-0208audit @ e664aa9a702bacac246096b6469e5d8cda6a3acd (main=e664aa9a).
Measured with: target-0208a/release ap-conform + antipythony (fresh `cargo build --release -p ap-conform -p ap-cli -p ap-aot-crt`, CARGO_BUILD_JOBS=4, ABSOLUTE CARGO_TARGET_DIR).

## 1. Coverage table (spec 05 §5.3 IN list = 53 top-level rows)

Source of truth: docs/spec/05-builtins-stdlib-v1.md §5.3; classification cross-checked
against `builtins_modules.rs::registry()` (31 entries), importer.rs special arms
(sys/builtins/_thread/_weakref/_typing/_frozen_importlib*), and vendored_stdlib/.

| Route | Modules | Notes |
| --- | --- | --- |
| NATIVE Rust registry (31) | sys, builtins, os, os.path, posixpath, genericpath, io, math, json, re (sre.rs engine), itertools, functools, collections, collections.abc, string, textwrap, heapq, bisect, operator, stat, errno, keyword, platform, time, atexit, linecache, struct (structmod.rs), tempfile, threading, weakref, importlib (+.util/.machinery/.metadata/._abc/._bootstrap) | native-first; 7 names (posixpath, genericpath, stat, keyword, linecache, collections, functools) are AUGMENTED with vendored pure-Python supplements (importer.rs:682, builtins_modules.rs:916) |
| Vendored pure-Python (22) | abc, argparse, ast (+native _ast bridge), codecs (+native _codecs + mbc*.rs CJK engines), encodings, enum, contextlib, copy, dataclasses, fnmatch, glob, inspect, numbers, pathlib, pprint, reprlib, token, tokenize (+native _tokenize), traceback, types, typing, warnings | exec from vendored_stdlib/ per DEVIATIONS.toml provenance |
| MISSING from v1 set | **none** | every spec-05 §5.3 IN row imports and smokes (52/52 pass; `sys` in-module face still xfail — test_t11 STILL_XFAIL_IN_MODULES={sys}) |

PARTIAL rows (S-05-4, 19): 12/19 pass. STILL-FAILING 6: sys.monitoring, sys.audit,
os.kill, os.pipe, ast.main, tempfile.mkstemp (all still-xfail; their xfail markers
are CORRECT). One row set is stale: the remaining STILL_XFAIL_PARTIAL_ROWS entries
now pass (inspect.FutureRef etc.) — flip-wave candidate, 01.02 protocol.

Registry-extras (spec-05 OUT names that HAVE native registry entries — deliberate
T1/T2 divergence, S-05 amendment territory): _collections, _imp, base64, binascii,
gc, hashlib, marshal, posix, shutil, unicodedata, zlib (A-2 wave). The vendored_stdlib/
dir additionally contains ~40 more OUT .py files (asyncio, csv, calendar, logging,
unittest, email, xml, ...) that import fine — see §3 OUT-face.

## 2. Gate: ap-conform stdlib slice (48 cases) — tier-0 vs adaptive vs binary

| Mode | Command shape | Pass | Fail | Timeout |
| --- | --- | --- | --- | --- |
| tier-0 | `antipythony --tier 0 run <case>` via ap-conform --impl | 30/48 | 16 | 2 |
| adaptive | same, no `--tier` prefix (runtime routes tiers internally) | 30/48 | 16 | 2 |
| binary | `antipythony build <case> -o <art>`; run artifact; compare expected triple | 29/48 | 17 | 2 |

Mode-equivalence (C-31): tier-0 vs adaptive are BYTE-IDENTICAL on every case except
heap addresses inside object reprs on 6 failing cases (s013/s014/s020/s025/s026/s042
— `<X object at 0x...>` from the SAME divergence, so mode-invisible). The binary leg
matches the interpreter fail-set exactly except s021_linecache (binary-only failure,
root-caused below). Verdict: **02.08 gate is RED** — 18/48 slice cases diverge.

Note (harness seam): `ap-conform run --impl '<cmd>' --tier 0` forwards `--tier 0`
AFTER the impl command (`<cmd> <file> --tier 0`), which `antipythony run` rejects
("unexpected argument '0'"). Passing the tier BEFORE the verb
(`--impl 'antipythony --tier 0 run'`) works; the T-11 pytest face does the same.
0/48-wide failure was harness-arg-order, not implementation.

### The 16+2 failing cases and root causes (both interpreter modes)

| Case | First divergence / error |
| --- | --- |
| s013 functools | update_wrapper: `__module__` None vs "builtins"; list-order `Rec([<obj 0x..>])` repr |
| s014 functools | lru CacheInfo `__repr__` missing (prints `<object at 0x..>`) |
| s015 collections | `Counter.__repr__`/most_common missing (dict print); `issubclass(Counter, dict)` False — Counter is not a dict subclass natively |
| s016 collections.abc | `isinstance(range, ByteString)`-row False + DeprecationWarning location `<antipythony>:1` vs file:line (warnings location attribution) |
| s017 types | `types.resolve_bases` / `prepare_class` missing; SimpleNamespace repr OK |
| s020 inspect | FullArgSpec `_replace`/repr missing; `inspect.BlockFinder` attr chain AttributeError (`function has no __globals__` upstream) |
| s022 tokenize | `tokenize.TokenError` not exposed (issubclass fails; vendored tokenize + native _tokenize split) |
| s023 ast | `ast.unparse` → ModuleNotFoundError `_ast_unparse`; Expression kwargs warning location |
| s024 enum | `NameError: __object_virtual___str____` inside enum member `__str__` (codegen mangle bug) |
| s025 dataclasses | default `P(x=1)` repr missing (dataclass `__repr__` not synthesized for that shape); `DC(1) != 5` TypeError |
| s026 typing | `_GenericAlias` repr + `__origin__/__args__` missing; `typing.Text`, `get_overloads` chain |
| s028 pprint | pprint dict ordering (`sort_dicts=True` default not honored) + pformat width wrap |
| s029 operator | `operator.not_` truth-result inversion; methodcaller `__name__` 'function' vs 'itemgetter'; missing `operator.<name>` attr |
| s030 copy | **HANG** (timeout 120s+). Root-caused: `iter(<genexpr>)` on a generator expression returns a wedged iterator — `next(iter(genexpr))` never returns (minimal repro: `args=(x for x in (1,)); next(iter(args))` hangs; `next(args)` works). copy.deepcopy hits it in `_reconstruct` (`args = (deepcopy(arg, memo) for arg in args)` then `func(*args)`). |
| s031 numbers | `isinstance(True, Integral)`-adjacent row False (bool ABC registration) |
| s036 struct | `heapq.heappush` on list containing Struct: `struct.pack_into` rejects non-bytearray buffer (native structmod buffer acceptance) |
| s042 argparse | `Namespace.__repr__` missing; subparser choice error text differs (`apx3: error` vs argparse format) |
| s044 pathlib | **HANG** (timeout) — same `iter(genexpr)` wedge (pathlib iterdir/glob chain uses it) |

### Binary leg extra failure

- s021_linecache: binary-only. Root cause: `__file__` in an AOT binary points at the
  ARTIFACT path (no .py), and source is not embedded (known 02.06 follow-up
  "stdlib .py not embedded yet") → `linecache.getline(__file__, 1)` returns ''
  instead of the `# Cites:` line. Interpreter mode passes (reads the real source).

## 3. OUT-face (S-05-22 fail-loud): NOT green

`test_out_module_build_fails_loud_with_token`: 51 modules xfail — `antipythony build`
SUCCEEDS (no APX-CEXT/APX-NOV1 refusal) for every one. Two mechanisms:
1. Native registry entries shadow the refusal (base64, binascii, gc, hashlib, marshal, posix, shutil, unicodedata, zlib + _collections/_imp — deliberate T1/T2 enablers).
2. vendored_stdlib/*.py found on the vendored sys.path imports fine (asyncio, csv, calendar, logging, unittest, email, xml, socket, ssl, subprocess, sqlite3 — scrapy/pytest chains need them).
This contradicts spec 05 §5.3.3/S-05-19..21 as written; it needs a SPEC AMENDMENT
(recording the T1/T2 enabler carve-out), not a code change — same shape as
Amendment A1 (2026-10-17).

## 4. Remaining-work list (priority order)

1. **FIX iter(genexpr) wedge** (crates/ap-run/src/iter.rs or interp.rs): `iter(x)`
   over a generator expression never advances. Blocks s030 AND s044 (2/48 slice),
   and likely much of copy/pathlib downstream code. Repro is 2 lines.
2. **Spec amendment: OUT-face carve-out** for the 11 registry-extras + vendored
   enabler modules (align spec 05 §5.3.3 with the T1/T2 reality; then decide
   whether the OUT fail-loud corpus face narrows to the still-refused set or the
   build verb gains an explicit allow-list policy).
3. **26 named surface gaps** (from §2 table): Counter-as-dict + Counter.repr,
   functools CacheInfo/update_wrapper.__module__, types.resolve_bases/prepare_class,
   inspect FullArgSpec surface + BlockFinder chain, tokenize TokenError export,
   ast.unparse (_ast_unparse), enum `__object_virtual___str____` mangle bug,
   dataclass __repr__ synthesis for default-shape, typing _GenericAlias
   __origin__/__args__/repr + Text/get_overloads, pprint sort_dicts/width,
   operator.not_/methodcaller.__name__, bool ABC (numbers) registration,
   struct pack_into buffer acceptance, argparse Namespace repr + error text,
   warnings location attribution (file:line vs `<antipythony>:1`).
4. **AOT source embedding / __file__** for linecache/traceback in binaries
   (s021 binary-only; known 02.06 follow-up, now gate-relevant for 02.08).
5. **Harness nit**: ap-conform `--tier` forwarding appends `--tier N` AFTER the
   impl command; `antipythony run` rejects it. Either forward before the verb or
   teach ap-cli to accept trailing --tier (1-line CLI pre-scan already exists for
   leading position).
6. **Flip stale xfail markers** (01.02): STILL_XFAIL_PARTIAL_ROWS entries that now
   pass (only sys.monitoring/sys.audit/os.kill/os.pipe/ast.main/tempfile.mkstemp
   still fail); `sys` in-module import-smoke remains the sole genuine IN-face xfail.

## 5. Reproduction commands

```
CARGO_TARGET_DIR=/Users/caimlas/.git/worktrees/leaf-0208audit/target-0208a   cargo build --release -p ap-cli -p ap-conform -p ap-aot-crt
cd /Users/caimlas/.git/worktrees/leaf-0208audit
ap-conform run --corpus 'tests/corpus/stdlib/*.py'   --impl 'target-0208a/release/antipythony --tier 0 run' --json --timeout 120
# binary: antipythony build <case> -o <art>; <art>; diff vs tests/corpus/stdlib/expected/
# pytest face (348 passed / 3 skipped / 69 xfailed / 8 xpassed):
AP_CONFORM=<...>/ap-conform APX_IMPL='<...>/antipythony --tier 0 run' PATH=<shim>:$PATH   /opt/homebrew/bin/python3.14 -m pytest tests/t11_stdlib -q -p no:cacheprovider
```
