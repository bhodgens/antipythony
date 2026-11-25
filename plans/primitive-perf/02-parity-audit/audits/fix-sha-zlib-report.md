# Fix report — leaf ppfix-perf: P05 sha256 asm accel + P04 decompress one-shot

Branch `tier1-ppfix-perf`, commit **884c535** on top of 4141a86.
Worktree: `~/.git/worktrees/leaf-ppfix-perf`. Bench outputs to scratch paths
(`/tmp/ppfix_perf_*.json`), shared `bench_report.json` untouched.

## Fix 1 — P05 hashlib.sha256 (asm-aarch64)

`crates/ap-run/Cargo.toml`: `sha2 = "0.10"` →
`sha2 = { version = "0.10", features = ["asm-aarch64"] }`. Verified the feature
name in the registry copy of sha2-0.10.9 (`asm-aarch64 = ["asm"]`, pulls
sha2-asm ARMv8 crypto-extension rounds; software path retained on non-aarch64
targets — no x86 backend in sha2-asm). Feature flag on an EXISTING dep; the
only new lock entry is `sha2-asm` (assembly source, no C toolchain).

| row | before (GB/s) | after (GB/s) | ratio before | ratio after |
|---|---|---|---|---|
| sha256 48KiB | 0.337 | 1.650 | 0.14x | 0.67x |
| sha256 1MiB | 0.358 | 2.207 | 0.14x | **0.88x** |

Standalone microbench: 0.34-0.36 → 2.07-2.40 GB/s (≈6x, matches the audit's
4-8x estimate). The remaining ~12% shortfall vs CPython's OpenSSL EVP at 1MiB
is probe overhead + EVP being marginally ahead of sha2-asm on M-series.

**Honest gaps (unchanged, per audit §3):** sha1 0.10.7 ships an EMPTY `asm`
feature (no aarch64 hw path; closing needs a new crypto dep = user decision).
md-5's asm is x86-only. sha3/blake2 have no ARMv8 CE instructions in any
RustCrypto crate — both sides run software and md5 is at 0.85-0.94x (parity
class). sha256 at 0.88x is parity-class but the harness verdict column still
reads FAIL (target = beat CPython as shipped).

## Fix 2 — P04 zlib.decompress (one-shot slice decode)

Three layers (the first two attempts were honest misses, recorded below):

1. **Loop shape.** `write::{Zlib,Deflate,MultiGz}Decoder` wrappers replaced by
   a single-pass slice decode: one `flate2::Decompress`, input re-sliced by
   `total_in()` each call (decompress_vec restarts its input cursor at slice
   start every call), `FlushDecompress::None` (Finish errors on mid-stream
   resume), Vec capacity grown 4x when saturated. A restart-from-scratch
   capacity loop was tried first and is 2.4x slower on compressible data
   (re-inflates the whole prefix per growth).
2. **Backend.** flate2 `zlib-rs` feature flag (zlib-rs 0.6.8 was already in
   Cargo.lock via a sibling; flag on an existing dep). miniz_oxide's inflate
   core tops out ~4 GB/s here; zlib-rs reaches libz-class 10-12 GB/s.
3. **gzip arm.** flate2's mem API has no gzip-header inflate, so wbits 25..47
   now parses RFC 1952 in-repo (`gzip_payload`: magic/method/flag, FEXTRA,
   FNAME, FCOMMENT, FHCRC) and inflates the raw payload one-shot with
   CRC32+ISIZE trailer verification (scalar `crc32_walk`). zlib.compress is
   UNTOUCHED (was already 1.5-8.4x PASS).

| row | before (GB/s) | after (GB/s) | ratio before | ratio after |
|---|---|---|---|---|
| decompress l1 48KiB | 2.070 | 6.208 | 0.49x | **1.33x PASS** |
| decompress l6 48KiB | 2.169 | 4.719 | 0.47x | **1.02x PASS** |
| decompress l1 1MiB | 3.565 | 10.843 | 0.42x | **1.26x PASS** |
| decompress l6 1MiB | 3.911 | 11.403 | 0.48x | **1.33x PASS** |

All four decompress rows now PASS (target >= 1x CPython); quiet-window confirm
run `/tmp/ppfix_perf_confirm.json`. Side effect: the zlib-rs backend also
lifted the compress rows (l6 1MiB 2.4x → 7.0x).

## Byte-exactness (gate first, always)

Differential probe `tests/bench/primitive/fix_sha_zlib.py` (818 output lines:
sizes 0..64 + 1KiB + 48KiB + 1MiB of a seeded deterministic byte file, zlib
roundtrips at levels 1/6 across wbits 15 / -15 / 47, all 9 native hashlib
algorithms, corrupt / truncated / garbage / bad-wbits exception texts; input
data pre-generated so interpreter loop-speed doesn't pollute the matrix —
generate `pfz_data.bin` with the xorshift in the probe docstring, default
path = probe dir). Green vs the rebuilt binary BEFORE the change (baseline
divergence set captured) and AFTER.

- Happy path (RAW digests, Z, RAWDEC, GZ, DONE): **409/409 identical** before
  and after.
- Total divergences vs CPython 3.14.7: **277 (pre-existing baseline) → 26**.
- All 26 are corrupt-mid-stream `zlib.error` reason texts on CORRUPT rows
  only: zlib-rs reports "repeated call with bad state" where libz reports
  "invalid distance too far back" (8 rows) / "incorrect data check" (18 rows).
  zlib-rs collapses two distinct libz messages; distinguishing would require
  guess-mapping. Recorded as an upstream message-granularity limitation.
- Gained along the way: WBITS-BAD error text fixed (out-of-range wbits now
  raises CPython's `Error -2 ... inconsistent stream state`, probed against
  3.14 for wbits 0/1/8/16/24/48/-1/-16/-8) and GARBAGE rows now match
  (277→26 includes those two families going green).

## Gates

- ap-conform tier 0: values **50/50**, stdlib **48/48**.
- `cargo test -p ap-run --release`: **208 passed / 0 failed / 1 ignored**
  (18 suites ok). Gotcha hit as documented: bugb_* suites spawn the target
  dir's debug-path bins under a release-only CARGO_TARGET_DIR — fixed with
  `debug/ap-run-eval`, `debug/ap-run-probe`, `debug/antipythony` symlinks →
  release bins (session-local, untracked, target dir is gitignored).
- Bench slice re-run with scratch `--out`; load < 10 window confirmed the
  final numbers above.
- `plans/robustness-ci/04-fuzz/FUZZ_SEEDS.txt` was appended to by a fuzz test
  during `cargo test`; reverted (not mine, not part of the leaf).

## Honest gaps / residuals

1. sha256 0.88x vs CPython (1MiB): parity-class, verdict column still FAIL —
   the harness target is "beat CPython as shipped"; closing fully means
   beating OpenSSL EVP, out of scope for a feature flag.
2. 26 corrupt-data error-text divergences (zlib-rs message granularity).
3. md5 0.85-0.94x, sha1 unaccelerated (no aarch64 asm upstream; new-dep
   decision), sha3/blake2 software-on-both-sides (unchanged).
4. The 1MiB decompress wins ride the zlib-rs backend; if the flag is ever
   reverted the loop-shape win alone was ~0.5x (2.1→3.6-4.0 GB/s, still FAIL
   vs CPython).
