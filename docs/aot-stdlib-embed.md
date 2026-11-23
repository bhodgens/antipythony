# AoT embedded stdlib — Gate Record (leaf tier1-linecache)

Measured 2026-11-23, worktree `~/.git/worktrees/leaf-linecache`
(branch `tier1-linecache`, base 663c216; commits 6d59fd1 + 8f66224). Machine:
Mac M2 Ultra, macOS 26.7 aarch64, rustc 1.94 (Homebrew), Apple cc/ld.
`CARGO_TARGET_DIR=$PWD/target-linecache`, `CARGO_BUILD_JOBS=4` (one heavy lane).

## What the leaf does

Closes 02.06 follow-up "stdlib .py not embedded yet" / 02.08 audit
remaining-work row 4 (**AOT source embedding / `__file__` for
linecache/traceback in binaries**):

- `xtask/gen_embedded_stdlib.py` → `crates/ap-run/src/embedded_stdlib/data.rs`:
  a static `include_bytes!` store of the vendored_stdlib `.py` sources —
  **465 files, 7,105,814 source bytes** (pydoc_data/ + _pyrepl/ excluded;
  keys are dotted import paths, byte-order sorted, `__init__` folded to the
  package name; package face wins over a sibling same-named module face;
  `collections.abc` stays absent — the importer synthesizes it from
  `_collections_abc.py`, exactly as on disk).
- `ap-run/src/embedded_stdlib.rs`: read side (binary_search `source_for`,
  the build-time vendored-dir label, store-sanity tests).
- `importer.rs`: `Found::Embedded` arm + `find_on_path`/`find_submodule`
  fallbacks that engage ONLY when `embedded_store_active()` — the build tree
  is absent or `ANTIPTYHONY_STDLIB` points elsewhere. With the tree present
  (repo runs) the resolution path is byte-for-byte the pre-leaf disk path.
  `compile_embedded` mirrors `compile_file`'s error conventions; embedded
  `__file__` labels keep the CPython-shaped `vendored_stdlib/<rel>.py` form.
- `builtins_modules.rs`: `embedded_source_for` falls through to the store
  for vendored labels the disk cannot serve → linecache
  `getline`/`getlines`/`updatecache` return the build-machine bytes
  standalone.

Native registry stays the FIRST face; disk wins when present;
vendored_stdlib/ untouched; ap-ce / ap-codegen-wasm untouched.

## Gate transcript

| Gate | Result |
|---|---|
| binary-mode stdlib slice (48 cases, build + run + diff-vs-expected triple) | **46/48** (audit baseline re-measured on this tree BEFORE the leaf: 46/48 — s021_linecache was already green via the 0206-embed entry-source seam; the 2 fails are s013 functools + s023 ast.unparse, the audit's DOCUMENTED interpreter-mode surface gaps, pre-existing, out of embedding scope) |
| s021_linecache through the binary | **byte-exact** (`getline(__file__,1).startswith("# Cites:")` + full expected triple) |
| standalone import face (`ANTIPTYHONY_STDLIB` redirected → store arms engage) | enum/warnings/inspect/dis/opcode/importlib chains resolve; module `__file__` keeps the vendored label; baseline (main) FAILS the same probe (ModuleNotFoundError) |
| standalone linecache face | `linecache.getline(enum.__file__, 2)` == disk bytes byte-exact standalone |
| hello + classes (c001) probes through the binary | **byte-exact** vs python3.14 (stdout+rc) |
| T-09 pinned 30-case slice through the BINARY | **30/30 inside 90 passed / 0 failed** (`pytest tests/t09_binary -q`, APX_CLI = leaf binary) |
| startup (soft gate ≤ 2x) | **0.87x** binary vs `run` (min-of-7: 7.1ms vs 8.2ms) — no regression |
| `cargo test --workspace --release -j4` | **PASS — 122 suites, 663 passed / 0 failed / 1 ignored, RC=0** (en-route: the documented fresh-worktree gotchas — built `ap-run-probe` to the baked `target/release/` path, debug `antipythony` + release `ap-host`/`ap-aot-crt-hybrid` for the build_ext/hybrid gates; one store-test fix, empty vendored `__init__` faces are legal) |

## Size delta (macOS aarch64, honest)

| Artifact | Bytes |
|---|---|
| hello-world binary, leaf (store dead-stripped — nothing imports) | 27,902,904 |
| hello-world binary, main baseline | 27,902,904 |
| enum-chain binary, leaf (store reachable) | 27,902,904 |
| enum-chain binary, baseline | 18,139,528 |

The store is `include_bytes!` data behind the importer fallback: a binary
whose import chains never leave the native registry dead-strips it (hello
delta = **0 bytes**); a binary that imports vendored pure-Python modules
carries it (enum-chain delta ≈ **+9.8 MB** over baseline). Full-store cost
upper-bounds at ~7.1 MB source bytes + index glue. The 15MB MUSL bound stays
report-only per T-09 GAP-2 (no musl toolchain on host).

## Known boundaries (documented, none silent)

1. The standalone state is verified via `ANTIPTYHONY_STDLIB` redirection +
   the store's unit tests; a true bare-machine run (no checkout at all)
   additionally exercises the `vendored_stdlib_dir() == None` arm — same
   code path, verified by the redirected probes.
2. `inspect.getsource` on vendored classes fails identically in tier-0 and
   standalone (Enum `__firstlineno__` surface gap — pre-existing, not an
   embedding regression; both modes byte-identical).
3. ap-conform `--impl-binary` was exercised indirectly via the T-09 pytest
   face (the harness's binary mode drives the same build+run+diff triple
   the manual slice used).
