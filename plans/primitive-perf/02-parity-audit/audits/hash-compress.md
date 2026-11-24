# Audit — P04 zlib/bz2/lzma + P05 hashlib

Leaf: 02-parity-audit/01-hash-and-compress. AUDIT-ONLY — no code edits, no commits.
Tree audited: worktree `leaf-primperf`, branch `tier1-primperf` (all cites below are
absolute-free paths relative to that worktree root).
Measured ratio column: **PENDING baseline (W2)** — `tests/bench/primitive/bench_report.json`
does not exist yet (00-harness in flight); orchestrator fills measured numbers at review.
Every claimed number for *CPython/crate speed* below that is not locally measured is
labeled **[est]** (knowledge-based, medium confidence) per the leaf contract.

---

## 1. Actual implementation state (verified, with cites)

### 1.1 zlib — native Rust, flate2 one-shot

- Registered as a native module: `crates/ap-run/src/builtins_modules.rs:872`
  (`("zlib", build_zlib)`); dotted-call routing at `:14775`.
- Surface: `crc32`, `adler32`, `compress`, `decompress` + constants + a real
  `zlib.error` class (`build_zlib` `:12057-12130`; dispatch `call_zlib` `:12137-12254`).
- Backend: **flate2 1.1.10** (`crates/ap-run/Cargo.toml:51`, plain `flate2 = "1"` —
  default features). Verified via `cargo tree -p ap-run -e features`: flate2 resolves
  features `any_impl,default,miniz_oxide,runtime_detection,rust_backend` → the
  **miniz_oxide 0.9.1 backend WITH `simd` + `simd-adler32`** (`miniz_oxide v0.9.1
  default,simd,simd-adler32,with-alloc`). `zlib-rs` 0.6.8 appears in Cargo.lock (via
  some sibling) but is **NOT compiled into ap-run** (`cargo tree -p ap-run` prints no
  zlib-rs edge; `--target all` grep empty).
- compress/decompress go through `flate2::write::{Zlib,Deflate,Gz,MultiGz}{En,De}coder`
  one-shots with wbits→container mapping (`:12159-12235`). `compressobj`/`decompressobj`
  raise `NotImplementedError` (streaming excluded, `:12238-12241`).
- `zlib.crc32`/`adler32` do NOT use flate2: they are **scalar byte-loop walks**
  (`crc32_walk` `:12266-12275` — per-byte, 8 shift-xor iterations each, ~200 MB/s class
  **[est]**; `adler32_walk` `:12277-12288` — scalar 5552-block loop). `binascii.crc32`
  (`:13088-13106`) duplicates the same bitwise walk.

### 1.2 bz2 / lzma — pure-python raise-shims (NOT implemented)

- Not native: no `bz2`/`lzma` entry in the module-registration table; grep of
  `builtins_modules.rs` finds zero bz2/lzma arms. No `bzip2`/`lzma` crate in
  `crates/ap-run/Cargo.toml`.
- They import via the embedded vendored stdlib:
  `crates/ap-run/src/embedded_stdlib/data.rs:83` (`bz2.py`) and `:365` (`lzma.py`).
  Both are 65/69-line shims: `compress`/`decompress`/file classes all raise
  `NotImplementedError("bz2 v0 shim: compression unimplemented")`
  (`vendored_stdlib/bz2.py:11-15`) and the lzma equivalent
  (`vendored_stdlib/lzma.py:14-20`). `encodings/bz2_codec.py` exists and would hit
  the same raise.

### 1.3 hashlib — native RustCrypto crates, one-shot

- Deps (`crates/ap-run/Cargo.toml:43-50`): `md-5 0.10`, `sha1 0.10`, `sha2 0.10`,
  `sha3 0.10`, `blake2 0.10`. Verified in Cargo.lock: md-5 0.10.6, sha1 0.10.7,
  sha2 0.10.9 (deps: cfg-if, cpufeatures, digest — **no sha2-asm**).
- Module build `build_hashlib` `:11658-11733`; algorithm dispatch `hex_digest_of`
  `:11734-11812` (md5 `:11732`, sha1 `:11738`, sha2 family `:11743-11761`,
  sha3 `:11761-11794`, blake2 `:11794-11806`); constructor surface `call_hashlib`
  `:11832+` incl. shake XOFs (`:11874+`), keyed blake2 (`:11912+`),
  `hashlib.new` (`:12000-12012`). Routing `:14773`.
- **Pure-python fallback: NONE** — every constructor name is native. The vendored
  `hmac.py` (`vendored_stdlib/hmac.py`) layers pure-python HMAC over it (`_hashlib`
  import fails → falls back), perf-irrelevant.
- Object model: hash objects are static attr bags — `hexdigest`/`digest` are
  **precomputed at construction** (`:11842-11843`), `update()`/`copy()` streaming
  does not exist. One-shot `hashlib.sha256(data).hexdigest()` is the fast path;
  an `.update()` probe would fail functionally, not slowly.
- **Hardware acceleration: OFF.** `sha2 = "0.10"` carries no features
  (`crates/ap-run/Cargo.toml:45`); sha2 0.10.9 exposes `asm-aarch64 = ["asm"]`
  (verified in the registry copy of sha2-0.10.9 Cargo.toml `[features]`) which pulls
  `sha2-asm` (ARMv8 crypto-extension SHA-2 rounds). md-5 0.10.6's `asm` is x86-only
  (`md5-asm`); sha1 0.10.7's `asm = []` is an empty/vestigial feature — no aarch64
  hw path exists in that crate.

---

## 2. Comparison references

### CPython 3.14 (local /opt/homebrew, 3.14.7 darwin — probed live)

- zlib: links the system libz; live probe reports `ZLIB_VERSION/ZLIB_RUNTIME_VERSION
  1.2.12`. macOS system zlib is Apple's build (derived from zlib 1.2.12) — on
  aarch64 it uses the ARMv8 CRC32 instructions for `crc32` **[est, medium]**. It is
  **stock zlib, NOT zlib-ng** — the leaf brief's "zlib-ng tier" does not apply to
  this host; the CPython floor here is stock-zlib tier **[est, high confidence:
  no zlib-ng in any macOS system libz]**.
- bz2 → libbz2 (bzip2 C library), lzma → liblzma (xz) — C-extension modules
  `_bz2`/`_lzma` (confirmed present in our own vendored sysconfig MODBUILT_NAMES,
  `vendored_stdlib/_sysconfigdata__darwin_darwin.py:730`; note CPython 3.14 also
  ships `_zstd`).
- hashlib: `_hashlib.HASH` (live probe confirms class), i.e. OpenSSL EVP. On
  Apple Silicon OpenSSL uses the ARMv8 crypto extension for SHA-1/SHA-2
  (SHA-256 ≈ 1-2 GB/s **[est]**); MD5 and SHA-3/Keccak and blake2 have no ARMv8
  CE instructions — those run software on both sides.

### Rust SOTA

- flate2 backends (verified from flate2-1.1.10 Cargo.toml.orig): default =
  miniz_oxide (+simd, +simd-adler32). `zlib-rs` feature = "the fastest backend
  overall" per flate2's own docs (zlib-ng tier, pure Rust + unsafe).
  `zlib-ng` via libz-ng-sys; system zlib via `libz-sys`.
- sha2 crate `asm-aarch64` feature: ARMv8 crypto-extension SHA-224/256/384/512 —
  the known Rust path to OpenSSL-class SHA-2 throughput **[est: ~1.5 GB/s vs
  ~150-250 MB/s software, 4-8x]**.
- crc32fast (already in our dependency tree via flate2/miniz_oxide: Cargo.lock
  `crc32fast v1.5.3 std`): slicing-by-16 + aarch64 `crc32x` hardware instructions
  — GB/s class **[est 2-6 GB/s]**.
- simd-adler32 (miniz_oxide's adler dep, not re-exported): NEON adler32 **[est 5-20x
  scalar]**.

---

## 3. Verdict table

"Ratio" = antipythony vs CPython 3.14 as shipped on this host; PENDING = W2 baseline.
Every row is explained for the P04/P05 bench families; bz2/lzma rows cannot appear in
bench_report.json today (both modules raise on call) — that absence is itself explained.

| primitive | our impl | our state | CPython floor (this host) | measured ratio | verdict |
|---|---|---|---|---|---|
| P04 zlib.compress (deflate, one-shot) | flate2/miniz_oxide+simd (`builtins_modules.rs:12159-12199`) | native, streaming excluded | stock zlib 1.2.12 | **PENDING baseline (W2)** | ALREADY-PARITY (pending confirm): miniz_oxide+simd ≈ stock zlib at default level **[est]**; if W2 shows >15% loss at level ≥8, upgrade to FIXABLE via `flate2` default-features-off + `zlib-rs` feature |
| P04 zlib.decompress (one-shot) | flate2/miniz_oxide+simd (`:12202-12235`) | native | stock zlib | **PENDING baseline (W2)** | ALREADY-PARITY (pending confirm): miniz_oxide decompress is at-or-above stock zlib **[est]** |
| P04 zlib.crc32 (+ binascii.crc32) | scalar bitwise walk (`:12266-12275`, `:13088-13106`) | native but ~10-20x off | Apple zlib w/ ARMv8 CRC ≈ 2-4 GB/s **[est]** | **PENDING baseline (W2)** | FIXABLE — see §4.1 |
| P04 zlib.adler32 | scalar 5552-block loop (`:12277-12288`) | native but slow vs hw | optimized adler (NEON-class) **[est]** | **PENDING baseline (W2)** | FIXABLE — see §4.2 |
| P04 bz2.compress/decompress | raise-shim (`vendored_stdlib/bz2.py:11-15`) | NOT IMPLEMENTED | libbz2 | n/a — no runnable row | NOT-WORTH for the perf gate (functional gap, not a perf gap); functional fix = new crate dep, user decision (§4.4) |
| P04 lzma.compress/decompress | raise-shim (`vendored_stdlib/lzma.py:14-20`) | NOT IMPLEMENTED | liblzma | n/a — no runnable row | NOT-WORTH for the perf gate; same functional-fix caveat (§4.4) |
| P05 md5 | md-5 0.10.6 software (`:11732-11737`) | native | OpenSSL EVP, software MD5 (no ARMv8 CE) **[est ~0.5 GB/s]** | **PENDING baseline (W2)** | ALREADY-PARITY (pending confirm): both sides software; md-5 is competitive **[est]** |
| P05 sha1 | sha1 0.10.7 software (`:11738-11743`) | native | OpenSSL EVP w/ ARMv8 CE **[est 1-2 GB/s]** | **PENDING baseline (W2)** | NOT-WORTH: likely a real gap **[est]**, but sha1 0.10.7 ships NO aarch64 asm (feature list verified: `asm = []` empty) — closing it needs a new crypto dep (ring/AWS-LC/OpenSSL) = user decision, and SHA-1 is a checksum/legacy role, rarely a hot loop in target workloads |
| P05 sha224/sha256 | sha2 0.10.9 SOFTWARE (`:11743-11755`) | native, hw-accel OFF | OpenSSL EVP w/ ARMv8 CE **[est 1-2 GB/s]** | **PENDING baseline (W2)** | FIXABLE — see §4.3 (the headline P05 finding) |
| P05 sha384/sha512 | sha2 0.10.9 SOFTWARE (`:11755-11761`) | native, hw-accel OFF | OpenSSL EVP w/ ARMv8 CE (sha512 rounds) **[est]** | **PENDING baseline (W2)** | FIXABLE — same flag as sha256 |
| P05 sha3_224/256/384/512 | sha3 0.10 software (`:11761-11794`) | native | CPython bundled Keccak reference C, software (no ARMv8 CE) | **PENDING baseline (W2)** | ALREADY-PARITY (pending confirm): both software Keccak, similar tier **[est]** |
| P05 blake2b/blake2s | blake2 0.10 software (`:11794-11806`) | native | CPython bundled blake2 reference C, software | **PENDING baseline (W2)** | ALREADY-PARITY (pending confirm): blake2 crate ≈ reference C tier **[est]** |

Counts: ALREADY-PARITY 5 · FIXABLE 6 · NOT-WORTH 5 (16 verdict rows; bz2/lzma split
compress/decompress but share one rationale each).

---

## 4. FIXABLE plans (exact change, est gain, est effort)

### 4.1 crc32 (zlib.crc32 + binascii.crc32) — biggest certain win in P04

- **Change:** replace `crc32_walk`/the inline binascii walk with `crc32fast`
  (`crc32fast::hash` for seedless; for the `value` seed map CPython's `!value`
  seed convention onto crc32fast's raw-state API — `crc32fast::Update` takes/returns
  the same inverted-register convention; must be verified by the differential gate,
  not assumed). crc32fast is **already vendored in Cargo.lock v1.5.3** via
  flate2→miniz_oxide; adding it as a *direct* dep of ap-run is a one-line
  Cargo.toml change and pulls no new source into the tree. Strictly, a new direct
  dep declaration → **user decision needed (trivial: already in the lock)**.
  Zero-new-dep alternative: hand-rolled slicing-by-8 table in builtins_modules.rs.
- **Est gain:** ~10-30x on crc32 rows **[est 0.2 GB/s → 2-6 GB/s hw on M2]**;
  closes the gap to CPython and likely exceeds it.
- **Est effort:** S (half a day incl. differential gate crc32(data, seed) chains
  vs CPython + bench re-run). Risk: seed-convention off-by-inversion — gate-first.

### 4.2 adler32

- **Change:** `simd-adler32` crate (NEON) — **NEW direct dep → user decision
  needed**. (It is already in Cargo.lock transitively via miniz_oxide but is not
  re-exported, so using it requires the direct declaration.)
- **Est gain:** 5-20x **[est]**; adler32 only matters if the bench slices it
  separately from compress/decompress (compress paths already get simd adler
  inside miniz_oxide).
- **Est effort:** S (half a day). NOT-WORTH if W2's bench doesn't isolate adler32.

### 4.3 SHA-2 family (sha224/256/384/512) — headline P05 finding

- **Change:** `crates/ap-run/Cargo.toml:45`
  `sha2 = "0.10"` → `sha2 = { version = "0.10", features = ["asm-aarch64"] }`.
  Feature flag on an **existing** dep — NOT a new dependency; sha2-asm 0.13 joins
  the lock (assembly source compiled in; no C toolchain). Non-aarch64 targets keep
  the software path (feature is aarch64-gated internally? NO — `asm-aarch64`
  implies `asm` = sha2-asm which is aarch64-only code paths; x86 builds must
  keep plain software: verify with a `cfg`-free build on host aarch64 only; x86
  gate runners use software automatically since sha2-asm has no x86 backend —
  confirm at impl time).
- **Est gain:** 4-8x on sha256 **[est ~150-250 MB/s → ~1.2-1.9 GB/s on M2 via
  ARMv8 sha256h]** — reaches parity with OpenSSL EVP (~1-2 GB/s **[est]**), the
  master.md P05 target. Byte-exactness: SHA-2 is a fixed function; asm and soft
  outputs are bit-identical, so the differential gate is low-risk.
- **Est effort:** S (one Cargo line + ap-conform hashlib differential slice +
  bench re-run; ~2-4 h). Only caveat: watch for force-soft fallback paths in
  debug builds (cpufeatures runtime detection still applies).

### 4.4 bz2/lzma (functional, out of perf-gate scope)

- bz2: `bzip2` crate (binds libbz2, C — matches CPython's floor exactly) or
  `bzip2-rs` (pure Rust, slower than libbz2). lzma: `lzma-rs` (pure Rust; slower
  than liblzma) or `liblzma` bindings. **All are NEW deps → user decision needed;
  do not assume any lands.** Also required regardless: streaming compressor/
  decompressor objects (BZ2Compressor/LZMADecompressor) for byte-exact chunked
  semantics — M-size work, not S.
- For THIS plan's exit gate these rows are NOT-WORTH: no bench row can even run
  against a raise-shim; the honest statement is "functional gap, priced
  separately, needs a user dep decision."

---

## 5. Open questions (honest-stop)

1. **Exact CPython zlib.crc32 throughput on this host** (does Apple libz actually
   use ARMv8 CRC on crc32?) — two probes spent (live zlib version pull + sysconfig
   inspection); left **[est]**. W2 baseline resolves it for free.
2. Whether W2's bench probes exercise one-shot `hexdigest()` only or `update()`
   chains — our hash objects have no `update()` (`:11837-11867`), so an
   update-shaped probe fails functionally. Flagged to the orchestrator: if the
   probe uses `.update()`, P05 rows are a functional exclusion, not a ratio.
3. sha2-asm behavior on non-aarch64 gate runners (macOS x86 CI?) — not verifiable
   from this host; verify at impl time.

## 6. Self-verification checklist

- [x] every P04/P05 bench family row explained (incl. why bz2/lzma have no rows)
- [x] each FIXABLE has exact crate/feature flag, est gain, est effort
- [x] every impl-site claim carries file:line, read and verified this session
      (registration :866-878, zlib surface :12050-12254, walks :12266-12288,
      binascii :13088-13106, hashlib :11655-12016, Cargo.toml :43-51,
      vendored bz2.py:11-15 / lzma.py:14-20, embedded data.rs :83/:365)
- [x] backend claims verified against the actual Cargo.toml + cargo tree feature
      resolution (miniz_oxide+simd confirmed compiled in; zlib-rs confirmed absent;
      sha2 asm features confirmed absent)
- [x] no implementation edits; only this doc written; nothing committed
