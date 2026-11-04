# Packaging — `antipythony new/add/run` (leaves 04.01 + 04.02)

Status: **implemented** (04.01 uv wheel-fetch + sys.path integration; 04.02 CLI
verbs + build cache). Plan text:
`plans/antipythony/04-packaging-ecosystem/orchestrator.md`. Test track:
`tests/t14_cli` (T-14).

## The flow

```console
$ antipythony new myapp          # scaffold: myapp/antipythony.toml + myapp/myapp.py
$ cd myapp
$ antipythony add six            # uv pip install --target <cache>/site-packages six
added six (six==1.17.0)
resolved with uv 0.11.29 (901092ee1 2026-07-15 aarch64-apple-darwin)
$ antipythony run myapp.py       # fetched deps join sys.path automatically
hello, antipythony
$ antipythony build myapp.py -o app
$ antipythony cache gc           # clear the build cache
```

## Cache layout

```
$ANTIPTYHONY_CACHE (default ~/.cache/antipythony)
  site-packages/      uv fetch target (04.01) — merged across adds
  build/<hash>/       content-addressed build entries (04.02)
  staging-<pid>-<n>/  transient; the pure-wheel inspection area for `add`
```

`ANTIPTYHONY_CACHE` overrides the root. `add` stages into `staging-*`, inspects
the tree for native components, and promotes into `site-packages` ONLY if pure
— a refused add leaves nothing behind (C-11 total refusal).

## uv integration

uv is used strictly as a **subprocess** (`uv pip install --target <dir>
--no-compile <spec>`) — never vendored or linked (uv is standalone Apache-2.0;
subprocess keeps our binary license-clean). Absent uv produces a helpful error
naming `uv` and the install docs
(`https://docs.astral.sh/uv/getting-started/installation/`). `ANTIPTYHONY_UV`
overrides the binary path.

Version pin (G-5: the leaf says "pin minimum version" without a number): the
code pins `UV_MIN_MAJOR = 0` in `crates/ap-pack/src/uv.rs` and echoes the live
version in the `add` output. Bump the constant when a feature requires a newer
uv; docs and code stay in sync by construction.

## Pure-wheel-only acceptance (add-time, fail early)

Two detection layers run over the staged tree BEFORE anything is promoted:

1. **File layer** — any member ending `.so` / `.pyd` / `.dylib` / `.dll` /
   `.a` / `.o` (same predicate as the T-14 fixture oracle).
2. **Tag layer** — any `*.dist-info/WHEEL` whose `Tag:` lines are not
   `*-none-any` (e.g. `cp314-cp314-macosx_...`).

A hit produces the C-07/C-08/C-10 diagnostic and exits nonzero:

```
APX-CEXT: `lxml` ships a native component (`/…/lxml/_elementpath.cpython-314-darwin.so`
is a C extension module); antipythony v1 refuses it. Pure-Python wheels only —
native components require Plan 10 (not yet implemented), see
docs/compat-contract.md C-07 and
plans/antipythony/10-c-extensions-stretch/orchestrator.md (/…:1:1)
```

The diagnostic carries, verbatim: the `APX-CEXT` token, the offending package
and the full native module path, the literal phrase `C extension`, both policy
pointers, and a `file:line:col` location (add-time refusals pin `:1:1`).

## Manifest + lockfile schema

`antipythony.toml` (the T-14 G-1 derived schema; fixture files in
`tests/t14_cli/fixtures/` are the oracle):

```toml
[project]
name = "myapp"
version = "0.1.0"
requires_python = ">=3.14"

[build]
backend = "native"
opt_level = 2

[[dependency]]
name = "six"
source = "file-cache"
```

`antipythony.lock` (packages sorted by name → deterministic across
re-resolutions; version pins come from the installed `*.dist-info/METADATA`,
not uv's human output):

```toml
[metadata]
lock_version = 1
created_by = "uv"

[[package]]
name = "six"
version = "1.17.0"
```

## sys.path ordering (04.02 run integration)

`bootstrap_sys_path` (crates/ap-run/src/importer.rs) builds, in order:

1. script dir
2. `vendored_stdlib/` (PSF-2.0 vendored stdlib — fetched packages can never
   shadow a stdlib module)
3. **fetched site-packages** — only when a project manifest (walking up from
   the script dir) records at least one dependency AND the cache dir exists
4. `ANTIPTYHONY_PATH` env entries (dev overrides win)
5. explicit extras (test seams)

Proof probe (`sys.path` dump from inside a run with `six` added):

```
/Users/…/myapp
/Users/…/antipythony/vendored_stdlib
/Users/…/.cache/antipythony/site-packages
```

Import resolution then flows through the existing file finder
(package/`__init__.py`, single-file modules, namespace packages, subpackage
descent, `from pkg import mod`).

## Build cache key (04.02)

`<hash>` = SHA-256 over (input bytes, lockfile bytes, toolchain version,
backend tag, opt level), NUL-joined. G-2 leaves the recipe unpinned — this is
our documented choice; tests assert entry-set behaviour only. `run --no-cache`
bypasses read and write. `cache gc` removes all build entries (G-8: eviction
policy unspecced; v0 clears everything).

## Known gaps (honest, queued)

- **SpecifierSet string/contains** — `str(SpecifierSet('>=1.0'))` /
  `.contains()` walk packaging 26's `_ranges`, which reads `cache_clear` off
  functions decorated through vendored `typing.py` runtime paths our typing
  stub does not fully serve (`AttributeError: 'function' object has no
  attribute 'cache_clear'`). `SpecifierSet`/`Specifier` CONSTRUCTION,
  `Version`, `Marker`, `Requirement`, and `canonicalize_name` are byte-exact
  vs CPython with the same package version; the remaining surface needs the
  typing-stub runtime completion (Plan-02 typing follow-up), not packaging
  work.
- **exec with explicit globals** — `exec(code, globals_dict)` fails loud
  (`NotImplementedError`); six only needs current-globals exec.
- **`run --wasm`** — rejected loudly (Plan 03 / leaf 04.04 territory).
- **`build` artifact** — the tier-0 binary emits a deterministic launcher +
  content-addressed staging entry; the true static artifact lands with 02.06
  AoT (byte-reproducibility across clean caches holds by construction).
