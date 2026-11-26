# Unsafe-Rust Assessment — issue #15

Measured at branch `tier1-assess15` (base main @ 8ed5f91), 2026-11-25.
Regenerate the raw counts with `scripts/unsafe_report.sh` (baseline committed
at `docs/unsafe-baseline.txt`). Small drift vs the issue body is expected and
explained: ap-run blocks 100 -> 101, ap-run extern/fns 24 -> 25 — exactly the
versioned-FFI bindings (cbrt@GLIBC_2.2.5, log@GLIBC_2.29 in
`builtins_modules.rs`) the issue's Notes section predicted.

## Per-crate measured category table (script output, src/ only)

| crate | unsafe { blocks } | unsafe fn + extern "C" | transmute | static mut | raw-ptr casts |
|---|---|---|---|---|---|
| ap-aot-crt-hybrid | 8 | 2 | 0 | 0 | 10 |
| ap-aot-crt | 7 | 2 | 0 | 0 | 9 |
| ap-ce | 94 | 309 | 6 | 2 | 38 |
| ap-cli | 0 | 0 | 0 | 0 | 0 |
| ap-codegen-native | 5 | 50 | 5 | 0 | 44 |
| ap-codegen-wasm | 0 | 0 | 0 | 0 | 1 |
| ap-conform | 0 | 0 | 0 | 0 | 0 |
| ap-gc | 11 | 2 | 1 | 0 | 18 |
| ap-host | 0 | 0 | 0 | 0 | 0 |
| ap-ir | 0 | 0 | 0 | 0 | 0 |
| ap-obj | 2 | 0 | 1 | 0 | 2 |
| ap-pack | 0 | 0 | 0 | 0 | 0 |
| ap-parse | 1 | 0 | 0 | 0 | 0 |
| ap-run | 101 | 25 | 9 | 1 | 55 |
| xtask | 0 | 0 | 0 | 0 | 0 |
| **TOTAL** | **229** | **390** | **22** | **3** | **177** |

The ap-codegen-wasm raw-ptr line is a safe `std::ptr::eq` identity comparison
in `lower.rs:1802` — no `unsafe` keyword, so `#![forbid(unsafe_code)]` holds.
`tests/` targets (108 blocks per the issue) are excluded: the script counts
`src/` only, matching the issue's per-crate figures.

## Representative site classification (33 sampled sites)

Categories per issue #15 work item 1:
(a) FFI boundary — unavoidable for a C-API + JIT product
(b) pointer/unchecked math safe stdlib now covers
(c) transmute replaceable by safe conversions
(d) static mut -> atomic/OnceLock
(e) suspect / genuinely-unavoidable-arch (asm, stack walk, JIT code ptr)

| # | crate | site | category | note |
|---|---|---|---|---|
| 1 | ap-run | builtins_modules.rs:3585 `sysmath::cbrt` | a | libc call |
| 2 | ap-run | builtins_modules.rs:3587 `sysmath::erfc` | a | libc call |
| 3 | ap-run | builtins_modules.rs:3949 `sysmath::erf` | a | libc call |
| 4 | ap-run | builtins_modules.rs:3968 `sysmath_ln::log@GLIBC_2.29` | a | versioned FFI binding (issue Notes) |
| 5 | ap-run | builtins_modules.rs:7665/7867/7887 `get_unchecked` (json scan) | b | bounds-checked index or iterator removes it; hot-loop perf choice |
| 6 | ap-run | builtins_modules.rs:9069 `libc::ioctl` | a | tty size |
| 7 | ap-run | builtins_modules.rs:9130 `libc::umask` | a | process state |
| 8 | ap-run | generators.rs:813 `libc::sysconf` | a | page size |
| 9 | ap-run | generators.rs:199 `(*y).suspend(...)` | e | raw GenFrame deref across yield boundary |
| 10 | ap-run | jit_hook.rs:877 `set_capture_point(caller_frame_pointer())` | e | fp capture for GC safepoint (asm-backed) |
| 11 | ap-run | jit_hook.rs:910 `tierup::call_fast(fast_ptr, ...)` | a | JIT-produced extern "C" entry |
| 12 | ap-run | rss_guard.rs:64 `libc::sysconf` x2 | a | phys mem probe |
| 13 | ap-run | threading.rs:631 `Arc::from_raw` | e | ownership handoff across thread boundary |
| 14 | ap-run | frame.rs:833 `libc::write` | a | panic-path fd write |
| 15 | ap-run | interp.rs:421/481/520 `transmute::<&ScopeInfo,&'static ScopeInfo>` | c | extend lifetime; arena/OnceLock-owned scopes make this safe |
| 16 | ap-run | sre.rs:1981 `transmute::<&Node,&'a Node>` | c | AST-lifetime extension; re-borrowable from owning Vec |
| 17 | ap-run | ce_bridge.rs:580 `transmute(init)` -> unsafe extern "C" fn | a | PyInit_* ABI dispatch |
| 18 | ap-run | aot_runtime.rs:109 `transmute(aot.code_ptr)` -> extern "C" fn | a/e | AoT entry jump — JIT-class, permanent |
| 19 | ap-run | simd_encode.rs:74/104/367/374 neon encode/decode | e | arch intrinsics; pinned preconditions |
| 20 | ap-gc | lib.rs:36 `transmute(f)` usize->fn (BRIDGE_DRAIN) | c | OnceLock<fn() -> usize> is a safe replacement |
| 21 | ap-gc | rootscan.rs:203/210 `asm!("mov {}, fp/rbp")` | e | frame-pointer read — no safe stdlib path exists |
| 22 | ap-gc | rootscan.rs:319 `fp_chain_walk(first_fp)` | e | walks the native stack; conservative-scan core |
| 23 | ap-gc | rootscan.rs:493–567 registry/slot reads (5 sites) | e | GC root bookkeeping over raw slots |
| 24 | ap-gc | gcmod.rs:919/944 `*(slot_addr as *const usize)` | e | payload read from collector slot addresses |
| 25 | ap-obj | gc_gate.rs:39 `transmute(f)` usize->fn (HOOK) | c | same OnceLock<fn> pattern as #20 |
| 26 | ap-obj | wtf8.rs:57 `from_utf8_unchecked` after manual encode | b | checked `from_utf8` + expect costs ~nothing off-hot-path |
| 27 | ap-parse | surrogates.rs:65 `from_utf8_unchecked` | b | same shape as #26 |
| 28 | ap-ce | exports.rs (26 sampled from 94) Py* fn bodies | a | every block calls/reads the CPython ABI |
| 29 | ap-ce | api.rs:214/226/229 `transmute::<InitRaw,PyCFunctionRaw>` | a | type-slot punning — CPython struct-layout ABI |
| 30 | ap-ce | api.rs:182/300 raw tp_* invocation | a | subclass slot calls |
| 31 | ap-ce | state.rs:250–271 per-interp state slot access | e | thread-local-ish state; candidate for OnceLock/refactor |
| 32 | ap-ce | exports_method_mem.rs:399 `static mut Py_Version` | d | C-ABI symbol export; needs the static mut for linkage but is write-once/never-mutated-in-practice — document, keep (FFI-constrained: `#[no_mangle]` const of that type is what C expects) |
| 33 | ap-ce | exports_pydantic.rs (18 blocks) | a | pydantic-band C-API shims |

### Sample sums vs measured totals (ap-run + ap-gc + ap-obj + ap-parse + ap-ce blocks)

| category | sampled (blocks touched) |
|---|---|
| (a) FFI boundary | 21 sites |
| (b) safe-stdlib-now | 4 sites |
| (c) transmute -> safe conversion | 4 sites (of 22 transmute mentions; remainder are comments — grep counts `transmute` word incl. doc text) |
| (d) static mut | 2 real decls (`Py_Version` exported const, +1 comment mention in gcsupport.rs) |
| (e) suspect/arch-unavoidable | 17 sites |
| unclassified remainder (not sampled) | 196 blocks = 229 total − 33 sampled |

(d) resolution: the third `static mut` grep hit is a comment in
`ap-run/src/gcsupport.rs:74` describing a *migrated* counter ("was a true data
race — now an AtomicU64"), so real statics = 2, both in ap-ce, both
C-ABI-exported.

## undocumented_unsafe_blocks backlog (clippy, `-W`)

`cargo clippy -p ap-obj -p ap-parse -p ap-gc 2>&1 | grep undocumented_unsafe_blocks | wc -l`
= **18 output lines** = **5 actual warnings** (ap-obj 1, ap-parse 0, ap-gc 4).
Full warn sweep: ap-run 7, ap-ce 1 missing-`// SAFETY:` warnings on the lib
target (most FFI bodies carry block-level SAFETY notes; test-cfg blocks aren't
counted by the lib build). The lint now lands workspace-wide as `warn` in
root Cargo.toml [workspace.lints.clippy]; flipping to `deny` is the follow-up
burn-down wave, not this leaf.

## Forbid rollout (this leaf)

- **ap-pack**: `#![forbid(unsafe_code)]` — measured 0/0/0/0, build+test green.
- **ap-host**: `#![forbid(unsafe_code)]` — measured 0/0/0/0, build+test green.
- **ap-conform**: `#![forbid(unsafe_code)]` — measured 0, build green (lib).
- **ap-codegen-wasm**: `#![forbid(unsafe_code)]` — 0 unsafe keyword (1 safe
  ptr::eq cast), build+test green.
- **ap-cli, xtask**: already at zero — DEFERRED, sibling/main-session own those
  crates per the worktree file-ownership rules. The ratchet script already
  pins them at 0 (and `--diff` excludes crates/xtask/ per its own hook policy).
- **ap-ir**: also measured zero; not in the issue's named six — available for a
  future wave, untouched here (outside declared file ownership).

## Verdict: the realistic floor

The issue's stance ("zero is not achievable for C-API/JIT/GC") is confirmed and
refined by the sample. The floor decomposes as:

1. **Permanent boundary unsafe (~195–210 blocks + all 390 extern/C sites).**
   ap-ce's C-API shim (94 blocks, 309 extern fns), the JIT/AoT entry calls
   (~5), GC frame-pointer asm + stack walking (11), libc syscalls in ap-run
   (~60 of its 101), cranelift-adjacent codegen (5+50), CRT glue (15), and
   NEON intrinsics (~10). None has a safe stdlib equivalent on stable today.
2. **Eliminable in follow-up leaves (~15–20 sites):** the fn-pointer transmutes
   in ap-gc/ap-obj registries -> `OnceLock<fn ...>` (2 sites, pattern repeats);
   the three `&'static` lifetime transmutes in ap-run interp.rs + sre::1981 ->
   owned/arena storage; `from_utf8_unchecked` x2 -> checked; `get_unchecked`
   hot-loops -> iterator/slice APIs where the differential gates prove no perf
   regression; ap-ce state.rs slot access -> runtime_state.rs rows (the 02.09b
   P3 pattern already exists).
3. **Suspect candidates warranting a fix wave (category e):** threading.rs
   `Arc::from_raw` handoff and generators.rs raw `suspend` deref — sound today
   per invariants documented in place, but they are the two sites where a
   future change could silently break soundness; prioritize SAFETY-comment
   hardening there first when `deny` lands.

Achievable end state (matches the issue): zero *undocumented*, zero *new*
(ratchet script + this leaf's clippy warn), unsafe confined to audited modules,
six+ crates hard-pinned via `#![forbid(unsafe_code)]`. Block count floors near
~210 across ap-ce/ap-run/ap-gc/codegen/CRT; the ratchet makes every future
reduction visible and every increase a red build.
