# AoT Static Binary — Gate Record (leaf 02.06)

Measured 2026-11-11, worktree `tier1-0206` (base d38e09b). Machine: Mac M2 Ultra,
macOS 26.7 aarch64, rustc 1.94.0 (Homebrew), Apple `cc`/`ld` (Xcode toolchain).

## What ships

`antipythony build app.py -o app` produces a native Mach-O arm64 executable:

1. parse → ap-ir (same front end as `run`);
2. every module-level `def` lowered through the 02.05 cfg stage;
3. one cranelift-object `ObjectModule` for all program fns
   (`per_function_section`, Export linkage) + `ap_aot_manifest` (one 64-byte
   row per fn: qualname ptr/len, entry-ptr function-reloc, num_slots,
   names-ptr/len, ic-ptr/len) + per-fn ExecCtx name tables + the embedded
   program source (`ap_aot_source` + length word);
4. system link (`cc` + `-Wl,-dead_strip`) against the `ap-aot-crt` staticlib
   (its `main`: rss-guard → manifest registration → catch_unwind →
   big-stack interpreter thread → `run_module` → exit code);
5. runtime: `call_function` dispatches main-module defs through the
   registered AOT bodies (same lowering + host seams as the JIT tier-0
   route); everything else tree-walks exactly as `antipythony run`.

Lowering-rejected module-level bodies are dropped from the manifest with a
loud `[AOT-BUILD] fn <name> tree-walks (cfg lowering rejected: ...)` build
notice and tree-walk at runtime — the 02.05 JIT fallback-table discipline
(SPLIT-02.05 accepted shape). Tier-0-NATIVE-EQUIVALENT = the same function
set native as the JIT route would compile, no more, no less. Remaining
codegen failures (object emit / verify / link) stay fail-closed build errors.

## Gate transcript

| Gate | Result |
|---|---|
| hello-world byte-identical vs CPython 3.14 | PASS (stdout/stderr/rc) |
| classes corpus program (c001) byte-identical | PASS |
| T-09 pinned 30-case slice through the binary | **30/30 PASS** |
| ap-conform `--impl-binary` over the slice | **30/30 PASS** |
| exit-code inline cases (11) binary == run verb | 11/11 parity (C-31 AoT invisibility) |
| of those 11, identical to pinned CPython triples | 7/11 — x003/x007/x008/x010 are PRE-EXISTING interpreter divergences (identical under `antipythony run`; not AOT bugs, out of leaf scope, corpus only grows) |
| PATH isolation (`PATH=/usr/bin:/bin`, no python3.14) | PASS (3 channels identical) |
| argv passthrough (`./app alpha 2` → `['alpha', '2']`) | PASS |
| cwd invariance (two working dirs, identical bytes) | PASS |
| startup (soft gate ≤ 2x JIT run) | **0.69x** (4.0ms binary vs 5.8ms JIT, min-of-5) |
| binary == forced-tier-0 JIT == tree-walk (same lowering) | 5/5 probes byte-identical |
| GC-heavy alloc probe (20M+ allocations in-binary) | PASS — roots + heap sizing live (review box 3) |
| catch_unwind safety net | CRT unit test `catch_unwind_maps_panic_to_internal_abort` PASS; panic → one-line abort message + rc 134, never a raw Rust panic print (review box 4) |
| `cargo test --workspace` | see PROGRESS.md (one heavy lane, CARGO_BUILD_JOBS=4) |

## Sizes (macOS aarch64, hello-world program)

| Artifact | Bytes | MiB |
|---|---|---|
| linked binary (dead_strip) | 17,723,816 | 16.90 |
| same, `strip -s` (symbol tables) | 14,307,384 | 13.64 |
| no-dead-strip control link | 18,971,744 | 18.09 |
| ap-aot-crt staticlib (input) | 31,910,800 | 30.43 |

Dead-strip saves ~1.2 MiB over the control. The bulk is the embedded
interpreter runtime — the native-module registry (`builtins_modules`)
references every native module unconditionally, so ~165 lxml/sre-adjacent
glue symbols survive `dead_strip` (verified with `nm`). **The 15MB gate is a
MUSL-static bound (orchestrator §02.06; T-09 GAP-2 makes macOS report-only).**
On musl the same link with `--gc-sections` plus a static libc typically lands
below the macOS number; measured musl numbers are BLOCKED (see below).

## Dead-strip accounting (review box 5, honest)

- NO stdlib `.py` payload is embedded: binaries resolve `vendored_stdlib`
  through the importer at runtime (build-machine path baked, the same
  documented Plan-04 packaging residual `run` already carries). Verified:
  `import json` works from cwd=/ with a scrubbed env.
- Native modules are registry-linked unconditionally; per-program
  dead-stripping of native module machinery requires making the registry
  lazy — future work (filed; would also cut the 165 surviving symbols).

## Targets

- **macOS aarch64 — PRIMARY VERIFIED** (all gates above).
- **linux x86_64 musl — BLOCKED (orchestrator decision needed)**: no musl
  cross toolchain on this host (`rustup` absent entirely — Homebrew rustc;
  `musl-gcc`/`musl-clang` not installed). The `--target linux-musl` flag is
  implemented and fails LOUDLY naming the missing toolchain; the emit +
  link paths are triple-parameterized (`AotTarget::LinuxMusl` → `-static
  --gc-sections`, PIC off). No software was installed per the machine-load
  policy; installing musl-gcc + the rust target is an orchestrator call.

## Known boundaries (documented, none silent)

1. `Stmt::For` / tuple-unpack `for a, b in ...` are outside the cfg stage's
   statement coverage → affected module-level fns tree-walk (loud NOTICE at
   build). Example: gc002's `make_chain` (its OUTPUT is still byte-identical
   via the tree-walk).
2. Traceback `File` lines in the binary show the BINARY's path (the program
   has no script file); matches the T-09 `<PATH>` masking convention.
3. `gc.get_threshold()` returns the runtime's (700,10,10) vs CPython's
   (2000,10,10) — pre-existing, identical under `run`.
4. ExecCtx slot-count and name-table rules are copied from jit_hook
   (`num_locals + max_temp`); drift between the two would be caught by the
   `aot_and_jit_share_slot_plans` unit test (same-cfg SlotPlan parity).
