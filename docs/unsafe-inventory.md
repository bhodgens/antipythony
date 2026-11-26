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

## Wave 2 — undocumented_unsafe_blocks burn-down (issue #18, branch tier1-ugw18)

Measured on `tier1-ugw18` (base main @ a38f490, which includes the sibling's
6880199 ap-ce lint-enable), 2026-11-26, via
`cargo clippy -p ap-obj -p ap-parse -p ap-gc -p ap-run -p ap-ce
--message-format=json` (primary-span counts — the issue's "13 warnings" was
stale pre-6880199; the real backlog was 115):

| crate | undocumented before | after | sites fixed |
|---|---|---|---|
| ap-obj | 15 | 0 | 15 (gc_gate.rs hook-fn transmute; 14 `unsafe impl Send/Sync` in markers.rs) |
| ap-parse | 0 | 0 | — (already clean) |
| ap-gc | 3 | 0 | 3 (gcmod.rs JIT-frame root-slot probes x2; lib.rs bridge-drain transmute) |
| ap-run | 0 | 0 | — (already clean) |
| ap-ce | 94 | 0 | 94 (11 files; see pattern breakdown below) |
| ap-codegen-native | 3 | 0 | 3 (path dependency surfaced by the `-p` graph: ic_cell bounds check, const-str slice, OSR fill) |
| **total** | **115** | **0** | **115** |

Fix shape: `// SAFETY:` comments ONLY (comment-only diff, +288 lines, zero
non-comment edits). Every comment states the concrete invariant, not a
placeholder — the recurring ap-ce patterns:

- **Minted-handle payload derefs** (majority): the handle is a
  `Box::into_raw`'d `CeObject` from `new_handle`/`from_value`/`module_create`
  that the ce layer never frees; payload `RefCell` borrows are short with no
  re-entrant borrow (ce dispatch stays inside the worker-serialization
  protocol — the same claim markers.rs pins).
- **S0 slot-fn signature-collapse transmutes** (api.rs/exports.rs): pointers
  come from the extension's `PyType_Slot` table compiled against the apx
  headers; on S0 the InitRaw/GetterRaw/SetterRaw signatures collapse to the
  PyCFunctionRaw `(self, args)` shape per `type_from_spec_normalized`.
- **Forwarding over the ce ABI** to in-crate unsafe extern-C surfaces whose
  documented preconditions the caller satisfies.
- **libc allocator seams** (PyMem family, payloads.rs): non-zero-size guards,
  ce-registry ownership, matching free seam.
- **Installed runtime fn-pointer seams** (errtb frame-query/matches/newexc,
  runtime_ctx errtb entry, exports pycall): `OnceLock` first-wins installs,
  called with pointers of the exact ABI types, valid for the call's duration.
- **FrameBuf fill contract** (frame_rows): CAP-slot storage, len clamped
  before `set_len`.

### Sites needing human review

None deferred — all 115 sites carry an honest justification; no site required
an invented claim. Two caveats recorded for the `deny` flip:

1. The ap-obj markers.rs Send/Sync comments and the ap-ce handle-lifetime
   comments both lean on the 02.09a P4/P5 staged-marker protocol argument
   ("interior mutation only under the worker-serialization protocol"). That
   protocol argument is a whole-system invariant, not a per-site proof; if the
   worker-serialization story changes, these comments (not the code) rot first.
2. `not_unsafe_ptr_arg_deref` (9 pre-existing errors, deny-by-default) still
   fires on `pub fn` sites in ap-ce state.rs/value_conv.rs/exports.rs that
   deref raw handle params without being marked `unsafe` — out of scope here
   (adding `unsafe` to signatures is a signature change, not a comment), and
   it predates this wave. It blocks any future `-D warnings` clippy run on
   ap-ce independently of undocumented_unsafe_blocks.

### Baseline discipline

`docs/unsafe-baseline.txt` regenerated in place: every category count is
UNCHANGED (comments only) — totals still 235 / 391 / 22 / 3 / 180. Gotcha
recorded: four SAFETY comments initially contained the literal words
`transmute` / `extern "C"` in prose, which the script's line-based category
greps counted (+3 transmute, +1 fn); they were reworded
("reinterpretation", "C-ABI fn") to keep the baseline byte-identical.
`scripts/unsafe_report.sh --diff` passes (no unsafe literals on added lines).

### Gates (all re-run at a8a3360)

- clippy undocumented_unsafe_blocks (-p ap-obj ap-parse ap-gc ap-run ap-ce):
  **115 → 0**
- `cargo build -p ap-run -p ap-gc -p ap-obj -p ap-parse -p ap-ce`: 0 errors
- `cargo test -p ap-gc`: 27+9 passed, 0 failed (GC-safety suites green)
- `bash scripts/unsafe_report.sh`: totals unchanged vs the pre-wave tree
- `cargo fmt -- --check` (ap-obj, ap-gc, ap-ce, ap-codegen-native): clean

Commits: a34ac01 (ap-obj + ap-gc + ap-codegen-native), 5730221 (ap-ce),
a8a3360 (comment rewording for baseline parity). No forbid attributes added
beyond what main already carries; no transmute/get_unchecked eliminations
(explicitly out of scope per the work order). The geiger CI step remains
orchestrator-owned (CI-matrix decision), untouched.

## Wave 3 — forbid rollout completion + clippy ratchet (issue #18, branch tier1-ugw20)

Re-measured 2026-11-26 on `tier1-ugw20` (base main @ 74febf1, which already
carries 6880199 + the merged wave-2 lineage). The issue body's "13 warnings"
was stale twice over (pre-6880199 it was blind to ap-ce; wave 2 burned 115):

undocumented_unsafe_blocks per crate (`cargo clippy -p <crate> --no-deps`,
counting "unsafe block without a safety comment"):

| crate | undocumented blocks |
|---|---|
| ap-cli | 0 |
| xtask | 0 |
| ap-ir | 0 |
| ap-run | 0 |
| ap-gc | 0 |
| ap-obj | 0 |
| ap-parse | 0 |
| ap-ce | 0 |
| ap-codegen-native | 0 |
| ap-codegen-wasm | 0 |
| (all others) | 0 |
| **total** | **0** |

Note: `cargo clippy -p ap-ce` still exits nonzero on 9 PRE-EXISTING
`not_unsafe_ptr_arg_deref` deny-by-default errors (pub fns deref-ing raw
handles without `unsafe` signatures) — documented in wave 2's "Sites needing
human review"; out of scope here and NOT a ratchet-lint count.

baseline: 0

### Forbid rollout (this wave)

6880199 pinned ap-cli's **bin** entry (`src/main.rs`) but missed ap-cli's
library target and its 4 eval microbench bins. This wave completes the pin —
each target verified zero-unsafe (`grep -c unsafe` = 0 outside the attribute;
clippy + unsafe_report.sh 0/0/0/0) before pinning:

| file | forbid added | verified zero-unsafe |
|---|---|---|
| crates/ap-cli/src/lib.rs | yes (this wave) | 0 blocks / 0 fns / 0 transmutes / 0 static mut |
| crates/ap-cli/src/bin/ap_gc_eval.rs | yes (this wave) | same |
| crates/ap-cli/src/bin/ap_mint_eval.rs | yes (this wave) | same |
| crates/ap-cli/src/bin/ap_obj_eval.rs | yes (this wave) | same |
| crates/ap-cli/src/bin/ap_run_eval.rs | yes (this wave) | same |
| crates/ap-cli/src/main.rs | already (6880199) | n/a |
| crates/xtask/src/main.rs | already (6880199) | n/a |
| crates/ap-ir/src/lib.rs | already (6880199) | n/a |
| crates/ap-ce | **excluded** | sibling lane owns the 109-warning fix wave; forbid deferred |

Holdout: none among ap-cli/xtask/ap-ir targets — all pinned. ap-ce is the
only remaining non-forbidden workspace crate with unsafe, by design (it IS
the C-API layer; floor documented in "Verdict: the realistic floor").

### Clippy-count ratchet (`--clippy` mode)

`scripts/unsafe_report.sh --clippy` (new): runs
`cargo clippy -p <workspace crates> --no-deps`, counts
"unsafe block without a safety comment" warnings, and exits 1 if the count
exceeds the `baseline: N` marker recorded above (or
`BASELINE_CLIPPY_UNDOCUMENTED` env override). Non-ratchet clippy errors
(e.g. the ap-ce deny lints) are noted but don't affect the count gate.
`CLIPPY_PACKAGES` overrides the default crate list.

CI (`.github/workflows/ci.yml` fast-lints job): new step
"unsafe-ratchet clippy gate — undocumented_unsafe_blocks drift (issue #18
wave 3)" invokes it with `continue-on-error: true` (informational first run
— flips to blocking once the ap-ce deny-lint cleanup lands) and prints a
visible `::warning` annotation on drift.

### Baseline regeneration

`docs/unsafe-baseline.txt` regenerated at wave-3 HEAD. Drift vs the wave-2
file is NOT from this branch: ap-codegen-native fns+externC 51 -> 54 and
rawptr-casts 45 -> 47 arrived with the merged hot-path-parity trunk
(clif.rs +1 fn, runtime.rs +2 fns, +2 rawptr casts — JIT lowering seams).
New totals: **236 / 394 / 22 / 3 / 183** (blocks unchanged).

### What remains

- ap-ce `#![forbid(unsafe_code)]`: impossible by design (C-API layer);
  the achievable end state remains the wave-2 verdict (zero undocumented,
  ratcheted, unsafe confined to audited modules).
- ap-ce `not_unsafe_ptr_arg_deref` (9 errors): blocks a future
  `-D warnings` clippy run; needs `unsafe fn` signature changes (sibling/
  follow-up lane).
- CI clippy step `continue-on-error: true` -> `false` once the above lands.
- geiger-style per-crate *category* trend reporting stays with the
  `scripts/unsafe_report.sh` default mode + baseline diff gate (6880199);
  this wave adds the clippy-count lane beside it.

