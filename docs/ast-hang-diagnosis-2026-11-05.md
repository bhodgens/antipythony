# AST hang diagnosis (read-only) — `import ast` + touching `ast.parse` hangs (rc=124)

Date: 2026-11-05. Tree: HEAD `743e277` + uncommitted W114 round-38 files.
Constraint honored: no antipythony binary was launched; this is pure code review.

## Verdict (root cause)

**Round-38 replaced `vendored_stdlib/_ast.py` with a byte-identical copy of
CPython's `ast.py` (md5 `bade0f45ce1f4587e8f52d60e7df9b07`, shared with
`vendored_stdlib/ast.py`). That file's first executable statement is
`import _ast as _ast_mod` (line 23) — a SELF-import — and the runtime's
`_ast` import branch inserts `sys.modules["_ast"]` only AFTER the body
executes. The self-import therefore re-enters the same file branch
unboundedly: recompile + re-exec + self-import, forever. There is no
recursion guard anywhere in the import path (grep for depth/recursion limits
in `crates/ap-run/src/` finds none on the call path). The process spins doing
quadratic registry work until the watchdog kills it → rc=124 = "hang".**

On CPython this is safe because `_ast` is a C extension pre-inserted into
`sys.modules`; the vendored file-branch design assumed `_ast.py` would be the
HEAD bridge module, which contains **zero import statements**
(`git show HEAD:vendored_stdlib/_ast.py | grep import` → no hits).

Secondary landmine (masks behind the hang): the round-38 `_ast.py` copy
**drops `from_native()`** (HEAD `vendored_stdlib/_ast.py:748`; absent in the
round-38 copy — `grep from_native vendored_stdlib/*.py` → no match). Even if
the import completed, `compile(..., PyCF_ONLY_AST)` would raise
`RuntimeError("_ast.from_native missing")` at `crates/ap-run/src/builtins.rs:4599-4600`,
so `ast.parse` still fails — as an exception, not a hang.

The signature "import `_ast` alone is clean" was observed against the
HEAD/clean-vendored stdlib (`/tmp/w116_t1cont/clean-vendored` holds the
round-38 copy too, so any probe pointed at `ANTIPTYHONY_STDLIB=…/clean-vendored`
or the main tree's `vendored_stdlib/` hangs on plain `import _ast`; the
"clean" rung predates the file swap).

## Exact code path (file:line each hop)

1. User script: `import ast` → `crates/ap-run/src/interp.rs:1226` (`Stmt::Import` arm) → `importer::import_module("ast")`.
2. `crates/ap-run/src/importer.rs:554` `import_module` → `:624` `import_module_inner("ast")`:
   - `:627` sys.modules miss; `:637` augment-list miss ("ast" not among the six); `:654` `_ast` branch miss; `:726` not dotted; `:743` native registry has no `"ast"` entry (registry table `crates/ap-run/src/builtins_modules.rs:758` registers only `"_ast"`).
   - Falls to the file finder → `import_file_module` → `Found::Module` (`:1195`) → `exec_compiled` (`:1216`).
3. `exec_compiled` inserts `sys.modules["ast"]` **pre-body** (`:1240`, "BEFORE body execution") — ast.py itself is circular-import-safe; then runs the body.
4. ast.py body line 23: `import _ast as _ast_mod` → `import_module_inner("_ast")`:
   - `:627` sys.modules **miss** (`_ast` is NOT pre-inserted on this branch),
   - `:654` the `if name == "_ast"` branch fires: `compile_file(_ast.py)` (`:439`) + `load_into` (`:468`) + `run_loaded` (`:528`) — and **`sys_modules_insert` happens only at `:671`, AFTER `run_loaded` returns successfully** (`:663`).
5. The round-38 `_ast.py` body line 23 is again `import _ast as _ast_mod` → re-enters step 4. **Infinite self-recursive module load.** Each level costs: read+parse+lower a 29 KB file, and `FnRegistry::extend_current` (`crates/ap-run/src/interp.rs:375-405`) clones the whole entries/by_path vectors (`:390-391`), so per-level work grows → the watchdog (30 s / 240 s fence) fires first; rc=124.
6. Independently, had the import finished: `ast.parse(...)` → `compile(src, fn, mode, 0x400, …)` → `crates/ap-run/src/builtins.rs:4592` ONLY_AST arm → `:4593` `ast_bridge::native_parse_tree` (works) → `:4598` `import_module("_ast")` → `:4599` `attr_get_public(…, "from_native")` → **None** on the round-38 copy → `:4600` `RuntimeError("_ast.from_native missing")`.

## Suspect-by-suspect findings (order from the handoff)

1. **Duplicate bytes-decode blocks, `vendored_stdlib/ast.py:78-88`** — REAL but benign: block 1 (`:80-81`) converts any `bytes`/`bytearray` to `str` unconditionally, so block 2 (`:84-88`) is dead code (its `isinstance(source, bytes)` can never be true). Not a hang. Collapse before committing, as the handoff says.
2. **bcc0a04 one-shot line index (`crates/ap-parse/src/lowering.rs`)** — EXONERATED. Index built once per Lowerer (`:239-244`, linear scan of `src`); `line_at` (`:590-596`) clamps `off` to `src.len()` and does `binary_search` over a strictly-increasing vector (`[0]` + one entry per `\n`) — safe for empty src (`line_starts == [0]`) and for src without a trailing newline. O(log n), no loop hazard. Also ap-parse is not part of the uncommitted diff at all.
3. **WIP `crates/ap-run/src/ast_bridge.rs` diff** — EXONERATED for the hang: the diff is rustfmt reflow plus the `posonlyargs` split (`:579-587`). No recursion shape; the bridge never calls back into the interpreter. The W110-5 closure-poisoning *name*-shape applies at `_ast.py` level (`parse`/`dump` defs in the upstream copy) but that manifests as a `RuntimeError`/wrong output, not a spin; the `dump = _dump_impl` alias workaround is preserved in both copies (ast.py diff `:162-165`, `:246-254`).
4. **codecs/encodings in the decode path** — EXONERATED: round-38 `ast.py`/`_ast.py` never import codecs (`grep` shows only `inspect`, `re`, `collections.deque`, `_ast_unparse`, `argparse` in lazy spots, none in the decode path). `bytes.decode` is native Rust (`crates/ap-run/src/methods.rs:1932 decode_bytes`; utf-8/latin-1 handled in-Rust at `:2002`, `:2089`) — no Python codecs registry lookup, no loop.

## Minimal repro (authored, NOT executed — run only through the fenced executor)

```python
# probe_ast_hang.py — fenced executor only (RSS cap 1GB, timeout 30s)
print("L1: start", flush=True)
import _ast                      # round-38 _ast.py self-imports at line 23 -> rc=124 here
print("L2: _ast imported", flush=True)   # unreachable iff hypothesis holds
import ast; ast.parse("x=1\n")   # L3: would raise RuntimeError(_ast.from_native missing)
```

Expected under the hypothesis: output stops after `L1`, killed at the 30 s
timeout (rc=124). Control: with the same binary pointed at
`ANTIPTYHONY_STDLIB=/tmp/w116_t1cont/wt-head`-style HEAD vendored tree, all
three lines print (L3 may print the from_native/TypeError diagnostics instead —
either way it does not time out).

## Recommended fix sketch (no code changed here)

1. **Restore `vendored_stdlib/_ast.py` to the bridge module** (HEAD's version:
   no imports, carries the class hierarchy + `from_native` at `:748` +
   `_NODE_TABLE`). Round-38's genuine wins (is_async ints, bytes constants,
   AsyncWith/AsyncFor, bigint literals, posonlyargs split) live in
   `ast_bridge.rs` and the dict→class converter, not in the upstream copy —
   port them onto the bridge file, do NOT vendor upstream `ast.py` as `_ast.py`.
2. Belt-and-braces on the importer: in the `name == "_ast"` branch
   (`importer.rs:654-678`), mirror `exec_compiled`'s pre-body insert
   (`:1240`) — create the (initially empty) module object and
   `sys_modules_insert("ast"==name, &v)` BEFORE `run_loaded`, so any future
   self-import resolves against the partial namespace instead of recursing
   (CPython circular-import semantics; the branch's post-hoc fresh-module
   rebuild at `:664-669` can keep overwriting with the finished ns).
3. Collapse the dead second decode block (`vendored_stdlib/ast.py:84-88`).
4. Keep `from_native` exported so `builtins.rs:4599` finds it; re-run
   `/tmp/w114_verify.py` under the fence, then commit round-38 as the single
   4-file pathspec commit.

## Files/lines inspected (evidence set)

- `HANDOFF-W114-2026-11-05.md:83-101`; `plans/antipythony/tests/W114-ROUND38-EVIDENCE.md:19-34`
- `vendored_stdlib/ast.py` / `_ast.py` (round-38): md5 identical (`bade0f45…`); line 23 `import _ast as _ast_mod`; `from_native` absent (grep rc=1)
- `git show HEAD:vendored_stdlib/_ast.py`: zero import statements; `from_native` at `:748`
- `crates/ap-run/src/importer.rs:554,624,627,654-678 (insert at :671 post-body),1195,1216,1240 (pre-body insert),439,468,528`
- `crates/ap-run/src/builtins.rs:4574-4601` (ONLY_AST arm; `from_native` requirement at :4599-4600)
- `crates/ap-run/src/builtins_modules.rs:758,1585` (native `build__ast` stub — shadowed by the file branch, never reached)
- `crates/ap-run/src/interp.rs:1226 (Import arm),375-405 (extend_current O(n) clone)`
- `crates/ap-parse/src/lowering.rs:234-296,573-596` (line index + `line_at`); `crates/ap-parse/src/lib.rs:74-96,193`
- `crates/ap-run/src/methods.rs:1932,2002,2089` (native bytes.decode — no codecs round-trip)
- `/tmp/w114_verify.py` (the fenced verify whose `dump_one.py` child is the rc=124 probe), `/tmp/w116_t1cont/{clean,dirty}-vendored` md5s
