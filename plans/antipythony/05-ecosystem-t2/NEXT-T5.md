# NEXT-T5 — Plan 05 T5 (comfyui/torch) reachability verdict

Date: 2026-11-23. Worktree: `~/.git/worktrees/leaf-t5` (branch `tier1-t5`,
off main `4fa6e56`). Target dir: `target-t5` (CARGO_BUILD_JOBS=4; one heavy
cargo lane held — build only, this is a measurement leaf). Measurement
artifacts: `scratch-t5/` inside the worktree (NOT committed; scratch), probe
scripts under `tests/t5-probe/`.

## VERDICT: **BLOCKED — three independent walls, none of them the C-API wall**

T5 (comfyui load + run a simple workflow) is NOT reachable on the landed S1
surface. The block is deeper than the binary-wheel question: the pure-Python
import chain dies inside **our own stdlib** before torch's C surface is ever
consulted. Kill-criterion language does not apply (T5 was always gated on
Plan 10 *full* C-extension support per master.md's tier table); this leaf
prices the gap.

Wall order (the order a comfyui import actually hits them):

1. **TYPING-SURFACE HANG (pure-Python, OURS — file as its own leaf, T4-adjacent).**
2. **importlib machinery degradation** (pure-Python surface, OURS).
3. **torch C-API demand 320 vs S1 233 covered → 186-symbol delta**, of which
   47 are S2-class (buffer/capsule/weakref/structseq/watchers) and 23 are
   S2/S4-class (threadstate/tss/frame-walking) — the *structural* floor.

### Wall 1 — `from typing import ContextManager` HANGS (repro bytes, prominent finding)

```
import sys
sys.path.insert(0, "/opt/homebrew/lib/python3.14/site-packages")
import typing                    # OK (vendored_stdlib/typing.py)
from typing import ContextManager   # HANGS (rc=124 at 15s/30s deadlines, zero output)
from typing import AsyncContextManager  # HANGS identically
from typing import AsyncGenerator     # OK
from typing import Generator          # OK
cm = typing.ContextManager            # HANGS (getattr route, same wall)
```

- `hasattr(typing, "ContextManager")` → False on our tree; on CPython 3.14.7
  `typing.ContextManager` exists and resolves instantly (the deprecated
  alias lives in `typing.__getattr__`, which consults
  `typing.extensions`/`_SpecialForm` machinery).
- Our vendored typing's `__getattr__` chain enters an unbounded loop when
  the name is NOT in `dir(typing)` — the alias-resolution path spins.
  CPython answers both names in <50 ms.
- **Why this is T4-relevant, not just T5**: fastapi/pydantic v2 typing
  chains import these deprecated aliases routinely (fastapi's
  `typing.ContextManager` usage is in the dependency wire). This is a
  **separate pure-Python bug leaf** — `typing.__getattr__` alias resolution
  must terminate (CPython-exact: resolve `ContextManager`/
  `AsyncContextManager` to the deprecated `_SpecialGenericAlias` + emit
  DeprecationWarning). File as its own leaf; do NOT bundle into a torch leaf.
- Repro file: `tests/t5-probe/t4_typing_contextmanager_hang.py` (byte-exact
  repro, runs under CPython 3.14.7 green / ours hangs).

### Wall 2 — importlib machinery degradation (pure-Python, OURS)

Probed while staging the torch import chain; three distinct defects:

- **F1 `str.splitlines(keepends=True)` DROPS line terminators** (pure-Py,
  byte-exact repro):
  ```
  s = "import abc\nimport builtins\n"
  ours:    s.splitlines(keepends=True) -> ['import abc', 'import builtins']
  cpython: s.splitlines(keepends=True) -> ['import abc\n', 'import builtins\n']
  ```
  Every terminator class drops (`\n`, `\r\n`, `\r`, `\v`, `\f`, `\x1c`,
  `\x85`, `\u2028`, `\u2029`). `bytes.splitlines` is entirely MISSING
  (`AttributeError: 'bytes' object has no attribute 'splitlines'`).
  Consequence: any `"".join(lines)` round-trip (importlib's own source
  handling, tokenizers, readlines-style parsing) silently corrupts source
  bytes. Repro: `tests/t5-probe/splitlines_keepends_repro.py`.
- **F2 `importlib.util.spec_from_file_location` returns a spec whose loader
  is an opaque `<object>`** — `exec_module` →
  `AttributeError: 'object' object has no attribute 'exec_module'`.
  `importlib.machinery` and `importlib.util` are NATIVE stubs, not the
  vendored `_bootstrap_external` classes: `im.SourceFileLoader.__module__`
  reports `__main__`, `__mro__` is `[SourceFileLoader, object]`, own
  `__dict__` EMPTY, and **calling the class raises
  `NameError: name 'SourceFileLoader' is not defined`** from inside the
  stub ctor. Repro: `tests/t5-probe/importlib_machinery_stub_repro.py`.
- **F3 `importlib.util.find_spec("torch")` returns None** even with
  site-packages FIRST on sys.path while `PathFinder.find_spec` under
  CPython finds it — our native finder stub fails to serve directory-based
  packages from arbitrary sys.path entries for some shapes (pure-py `yaml`
  DOES import; `torch` does not). Net: the manual
  `SourceFileLoader.exec_module` staging route (how a wheel would be
  staged without pip) is unavailable end-to-end.

These three mean **"stage comfyui's pure-Python portion per its
requirements" cannot complete through the import machinery itself** —
comfyui's chain (typing_extensions → torch → …) wedges at Wall 1/2 before
any C surface matters.

### Wall 3 — the torch binary demand (nm inventory, the C-API half)

torch staged from the LOCAL install `/opt/homebrew/lib/python3.14/site-packages`
(torch 2.13.0, cp314 mac-arm — same wheel class the survey measured; no
download needed, wheel download was denied this session). The `import
torch` chain dlopens `libtorch_python.dylib` (the `PyInit__C` bundle
`_C.cpython-314-darwin.so` itself only forwards; ALL C-API demand lives in
the dylib).

Measured (plain `nm`, undefined symbols; `nm -U`/`-D` are broken on
Mach-O bundles here — survey §method notes the same):

- libtorch_python.dylib undefined `Py*`: **320**
- antipythony binary exported `Py*` surface (landed S1 + spike + pydantic
  band): **233**
- **covered: 134; delta: 186** (files: `scratch-t5/torch_undef_Py.txt`,
  `torch_delta.txt`, `torch_delta_buckets.json`)

Delta classification (full list in the buckets json):

| bucket | n | class |
|---|---|---|
| S1-wrapper (call/long/unicode/seq/map/iter/slice/number/module/bytes) | 85 | S1-class, vtable/direct wrappers per the 1005b/pydantic model — mechanical |
| S1-token (26 `_Type` tokens + 21 `PyExc_*` data) | 47 | S1-class identity tokens per the 1006 pattern |
| S1-era (`PyObject_{Clear,Visit}ManagedDict`, `PyUnstable_{Code_*,TryIncRef,EnableTryIncRef,Eval_RequestCodeExtraIndex}`) | 7 | S1-class (3.14 dialect / PyO3 emissions) |
| **S2-buffer** (`PyObject_GetBuffer`, `PyBuffer_{IsContiguous,Release}`, `PyObject_CheckBuffer`, `PyMemoryView_{FromBuffer,FromMemory,FromObject}`) | 7 | **S2 structural** — buffer protocol depth (survey §2 S2 row) |
| **S2-capsule** (`PyCapsule_{New,GetPointer,GetName,IsValid,Set*,Get*}`, `PyCapsule_Type`) | 8 | **S2 structural** — capsule surface |
| S2-weakref (`PyWeakref_{NewRef,GetRef}`, `PyObject_ClearWeakRefs`) | 3 | S2 structural |
| S2-structseq (`PyStructSequence_{InitType,New,SetItem}`) | 3 | S2 structural (os.stat-class shapes) |
| S2-dict-watchers (`PyDict_{AddWatcher,Watch,Unwatch}`) | 3 | S2 structural |
| **S2/S4-threadstate** (`PyThreadState_{New,Clear,DeleteCurrent,Get,GetDict,GetFrame,GetUnchecked,Next}`, `PyThread_tss_*` ×4, `PyInterpreterState_{Get,GetDict,ThreadHead}`, `PyGILState_GetThisThreadState`, `PyEval_AcquireThread`) | 17 | **S2/S4** — thread-state introspection + TSS keys; our no-GIL runtime models these as bookkeeping (survey §4/§5 commitment 4) but the surface is unlanded |
| S2/S4-frame-eval (`PyCode_Addr2Line`, `PyEval_{GetBuiltins,GetFrame,GetFrameLocals}`, `PyEval_SetProfileAllThreads`) | 5 | S2/S4 frame-walk/profile hooks |
| S2/S4-arena (`PyObject_GetArenaAllocator`) | 1 | S4 embedder tier |

**Summary: 139 of the 186 (75%) are S1-class wrappers/tokens — mechanical
under the established models. 24 are S2-core structural (buffer depth is
the real work: torch's tensor export path is buffer-protocol-native). 23
are S2/S4 thread-state/frame surface.**

### What this means for the S2/S3 tier list (survey §2 cost-class)

- **S2 core-C floor** (S1 + static-`PyTypeObject` recompile-header story +
  buffer protocol depth + tp_slots + weakrefs + capsules + datetime) is
  CONFIRMED load-bearing for torch: the measured delta demands 24 S2-core
  members, buffer protocol first among them.
- **S3 numpy tier** is NOT yet reached by this measurement — comfyui's
  chain also requires `numpy` (874-symbol package per survey §1) and
  torch's own `PyCapsule_Import`-shaped internal C surface beyond
  `libtorch_python`. The numpy demand sits on top of this delta and is
  priced separately per survey §2.
- comfyui tier table row said "torch and friends — the heaviest
  binary-wheel stack": measured. The C-API half alone is ~186 symbols
  (139 cheap + 47 structural); the pure-Python half (Walls 1–2) is
  currently the FIRST thing that breaks and is fully in our own tree.

### Honest gaps / scope notes

- torch 2.13.0 measured, not 2.15.0 (wheel download denied this session;
  the installed cp314 wheel is the same demand-class — survey caveat:
  "symbol demand is demand regardless of build tag").
- `import torch` was NOT made to reach the dlopen stage through our
  importer: the hang at Wall 1 fires first (typing_extensions imports
  `ContextManager` aliases inside torch's chain via typing). A manual
  `ctypes.CDLL` dlopen of `_C.cpython-314-darwin.so` DOES succeed (rc=0,
  0.0s — the bundle loads; its undefined `Py*` resolve against the
  exporting binary is the S1 contract and 134/320 are present). The
  symbol-wall failure mode for the remaining 186 is therefore predicted
  (dlopen undefined-symbol abort naming the first missing `Py*`), not
  byte-captured end-to-end — the import machinery would have to be
  repaired first to capture it through `import`.
- comfyui-without-torch entry does NOT exist: `main.py` line 1 is
  `import comfy.options` (pure, WORKS: RC=0 through the binary), then
  `folder_paths` (pure, WORKS: RC=0), but every node-loading path
  (`nodes.py`, `comfy/` internals) imports `torch` unconditionally.
  Recorded honestly: the pre-torch leaves of the chain are green; the
  chain dead-ends at torch.
- No production code touched. Findings F1/F2/F3 + Wall 1 are
  **report-to-orchestrator** items with repro files.

## Files (committed)

- `plans/antipythony/05-ecosystem-t2/NEXT-T5.md` (this file)
- `tests/t5-probe/t4_typing_contextmanager_hang.py` — Wall-1 repro (hangs
  on ours, green on CPython)
- `tests/t5-probe/splitlines_keepends_repro.py` — F1 repro
- `tests/t5-probe/importlib_machinery_stub_repro.py` — F2 repro
- `tests/t5-probe/torch_stage_probes.py` — stage-1/2/3 probes (comfy.options
  RC=0, folder_paths RC=0, torch import wedge capture)
- `tests/t5-probe/torch_symbol_delta.json` — 186-symbol delta + buckets +
  covered/total counts (machine-readable)

## Cost-class estimate (per survey §2 tier table)

- To UNWEDGE the pure-Python chain: 3 leaf-sized fixes (typing alias
  getattr termination, splitlines keepends + bytes.splitlines,
  importlib.util/machinery un-stubbing or honest failure modes). Each is
  T4-blocking too — schedule ahead of T5.
- To pass Wall 3 for real: land the 139 S1-class members (measured band:
  10–15 min/fn wrapper + token pattern — the 1003 note's band) ≈ 25–35 h
  mechanical; then S2 buffer protocol + capsule + weakref + structseq +
  watchers ≈ the survey's "+~120" S2 row, of which torch needs the 24
  named above; then S2/S4 threadstate/frame surface (23 syms) rides the
  PEP-703 bookkeeping commitment. numpy (S3) is a separate price on top —
  torch/comfyui does NOT become reachable at S1+S2 alone unless numpy is
  included, because the chain hard-requires numpy.
- Verdict class: **BLOCKED — reach requires S2-core landed + S3 numpy tier
  priced + 3 pure-Python wedge fixes, of which the typing hang is the
  cheapest and most broadly valuable (fix T4-blocking first).**
