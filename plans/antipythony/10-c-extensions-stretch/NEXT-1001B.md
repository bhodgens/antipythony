# NEXT-1001B — Leaf 10.01b session record (2026-11-20)

## What this leaf was
Completes the crashed Leaf 10.01 (C-extensions survey). The sibling's measurement
data survived the disk-full crash: `/tmp/ce-scan/results.json` (49-package mac-arm
wheel inventory) + `docs/measure-ce-surface.py`/`docs/wheel-scan-results.json`
(pass-1 manylinux scan, committed to leaf-1001's tree). This leaf re-measured the
undefined-symbol surface (`nm -D` fails on Mach-O; recovered with plain `nm -U`),
ran gap-fill re-scans, and wrote `docs/ce-compat-survey.md`.

## Measured (final)
- **40 packages with native code + measured Py\* imports; union = 1,676 symbols**
  (core ≥40% of pkgs: 147; mid: 355; tail <2 pkgs: 1,174, numpy-C-API-dominated).
- True CPython C-API union (ex package-private namespaces): **704 syms; 487
  stable-ABI (69.2%) + 73 public-non-stable** (PyO3 emissions 18, frame/code 8,
  method ctors 7, 3.14-era additions ~40). S0+S1 target ≈ **560 members**.
- **pydantic-core 2.50.5**: 169 Py\* imports, 92.9% stable; 11 non-stable, all small
  (`PyUnicodeWriter_*` family, `PyUnicode_New/DATA`, `PyObject_CallOneArg`,
  `PyObject_VectorcallDict`, `PyObject_LengthHint`, `PyFunction_Type`); 100%
  FromSpec-era type creation; polite GC subset. Pass-1 (2.50.4 manylinux) agreed.
- Rust/PyO3 class = 9 of 40 pkgs (pydantic-core, cryptography, tokenizers, tiktoken,
  orjson, bcrypt, safetensors, rpds-py, watchfiles) — one shared surface (S1).
- Gap-fill verdicts: pydantic/argon2-cffi/pymerkle = pure-python wheels; simdjson =
  no mac-arm wheel (sdist only); gevent 300 syms/79% stable (its own 13
  `PyGevent*` types + static inlines); greenlet 83 syms/90% + stack-switch special
  case; bcrypt cp39-abi3 = 100% stable (cleanest client); grpcio 226/91.2%.

## Verdict
- **Recommended strategy: new extension ABI + CPython-source-compatible headers +
  ap-obj-backed source-shim** (rebuild-required; binary wheels refuse loudly).
  Full C-API emulation rejected (layout replication = PyPy/GraalPy refusal);
  pyo3-direct-integration named as fallback only (serves 9 pkgs, not the 31 C/C++).
- **pydantic-core verdict: REACHABLE on S1** → T3 (hermes-agent) / T4 (fastapi)
  reachable, contingent on S1 landing (~560 measured members, zero layout deps).
- **Kill-criterion** (fires at 10.02 gate): (1) S0 (~120 fns incl. FromSpec+slot
  trampolines+vectorcall) must land within 6 weeks AND S0+S1 density < ~2
  person-years; (2) minimal PyO3-shaped module must import/instantiate/round-trip
  on S0 (RustPython-#8435 failure mode); (3) trivial pydantic model must validate
  on S1 — else STOP / re-scope to S2-only.

## Commit
- `0488afd` on `tier1-1001b` (worktree `~/.git/worktrees/leaf-1001b`, off main
  `4b8e1ee`): survey doc + scan_wheels.py + ce-scan-results-macarm.json +
  ce-scan-symbols-full.json + ce-scan-tiers.json + ce-scan-gapfill.json; pass-1
  artifacts (measure-ce-surface.py, wheel-scan-results.json) already on main,
  unchanged. Docs-only; no code touched; parallel-safe (read-only for siblings).

## Notes / gotchas for the next session
- `nm -D` errors on Mach-O ("File format has no dynamic symbol table") — use
  plain `nm` and read `U` lines. This is why the sibling's results.json had empty
  undefined_Py lists; recovered lists live in ce-scan-symbols-full.json.
- stable-ABI classification via `abi3info` 2026.9.25 (`FUNCTIONS`/`DATAS` name
  sets; total 968 members). numpy-class packages score low (39–79%) only because
  their own C-API namespaces (`PyArray*`, `PyImaging*`, `PyGevent*`) are counted
  against CPython's list — they're package surface, not CPython surface.
- /tmp/ce-scan and /tmp/ce-scan2 are volatile; all data needed is now committed.
