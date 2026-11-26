# fix-bz2-lzma report (leaf pp2-bz2)

Branch `tier1-pp2-bz2` @ 86b6e0e. Replaces the bz2/lzma NotImplementedError
raise-shims with real one-shot module surfaces backed by pure-Rust crates.
Crate dependency approval was pre-granted by the user (bzip2 / lzma-rs).

## 1. Research: crate options and the C-vs-pure-Rust decision

### bz2

| crate | backend | C dep? | encode | verdict |
|---|---|---|---|---|
| `bzip2` 0.6.1 (Trifecta Tech) | **libbz2-rs-sys 0.2.7 — default** | **none** | yes | **CHOSEN** |
| `bzip2` w/ `bzip2-sys` feature | C libbz2 built from source | cc build | yes | rejected (C dep) |
| `bzip2-rs` 0.1.2 (paolobarbolini) | pure-Rust decoder only | none | **no encoder** | rejected: no compress |

Key finding: since bzip2 0.6.0 (2025-06), the `bzip2` crate's default backend is
`libbz2-rs-sys`, a **c2rust translation of the same C source CPython links**
(Radically Open Security-audited; Trifecta's benches show it 5–15% *faster*
than C on decompress). This gives libbz2-exact byte compatibility in both
directions with **zero C toolchain involvement** — the repo's no-C-deps build
story (CRT only) is preserved. The `bzip2-sys` C backend remains opt-in and is
NOT enabled. License: the `libbz2-rs-sys` crate carries the bzip2-1.0.6
BSD-style license (derivative-work designation); `bzip2` itself is MIT/Apache.
`THIRD_PARTY_NOTICES` regenerated (323 deps), `xtask notices --check` OK.

### lzma

| crate | backend | C dep? | xz+alone+raw | verdict |
|---|---|---|---|---|
| `lzma-rs` 0.3.0 | pure Rust (safe, `#![forbid(unsafe_code)]`) | none | yes | **CHOSEN (user-approved)** |
| `liblzma` 0.4.9 | C liblzma via liblzma-sys (cc/pkg-config) | **C build** | yes | rejected (C dep) |
| `xz`/`xz-core` 0.4.6-rc | pure-Rust c2rust port of xz | none | yes | strong future swap (below) |
| `xz-native` 1.1.0 | pure Rust, liblzma-5.8.4-byte-identical encoder | none | yes | too new (created 2026-10, ~1k downloads, 0 dependents) |
| `lzma-rust2` 0.26 | pure Rust (tukaani xz-for-java port) | none | partial | API lacks alone-header control used here |

`lzma-rs` is the conservative pick: 100% safe Rust, MIT, in the offline
registry cache. **Recorded tradeoff**: its range-encoder is a reference
"literal-only" encoder, so compress OUTPUT diverges byte-wise from liblzma
while remaining container-valid and decompressible by CPython; its xz
container writes check=NONE (CPython default is CRC64). The DECODE side —
the hard-exact contract — accepts everything CPython produces (CRC32/CRC64
block checks are verified). If byte-identical xz compress output ever becomes
a gate, the `xz`/`xz-core` crate (c2rust port of xz internals, fork of
liblzma-rs, backend-swappable) is the drop-in path; `xz-native` is the
byte-identical-encoder option once it matures.

## 2. Byte-exactness matrix

| surface | direction | exactness |
|---|---|---|
| `bz2.compress(data, level)` | ours → CPython | **BYTE-IDENTICAL** (same encoder math; pinned hex in the gate) |
| `bz2.decompress(CPython bytes)` | CPython → ours | **EXACT** (incl. multi-stream concat, empty→b'', all 9 levels) |
| `bz2` error texts | both | CPython-exact (OSError 'Invalid data stream'; ValueError end-of-stream-marker; compresslevel 1..=9) |
| `lzma.decompress(CPython xz/alone/raw)` | CPython → ours | **EXACT** (check CRC32/CRC64 verified; preset 0/6/9, 9e, filters=) |
| `lzma.compress` | ours → CPython | decompress-compatible ONLY (container valid, byte-wise divergent; xz check=NONE vs CRC64) |
| `lzma` error texts | both | CPython-exact (probed table below) |
| `lzma` constants | — | exact values (FORMAT_*, CHECK_*, PRESET_EXTREME=2^31, FILTER_LZMA2=33, FILTER_LZMA1=1<<62|...) |

Error-text table (all probed against /opt/homebrew/bin/python3.14, gate-pinned):
`OSError 'Invalid data stream'`, `ValueError 'Compressed data ended before
the end-of-stream marker was reached'`, `ValueError 'compresslevel must be
between 1 and 9'`, `LZMAError 'Input format not supported by decoder'`,
`LZMAError 'Compressed data ended before the end-of-stream marker was
reached'`, `LZMAError 'Corrupt input data'`, `ValueError 'Must specify
filters for FORMAT_RAW'`, `ValueError 'Invalid container format: N'`,
`LZMAError 'Invalid or unsupported options'`, `ValueError 'Integrity checks
are only supported by FORMAT_XZ'`, `LZMAError 'Internal error'`.

## 3. Gate results

`tests/bench/primitive/fix_bz2lzma.py` — 62 stdout lines, **byte-identical
under CPython 3.14 and antipythony (62/62, 0 diffs)**:

- roundtrips at 0 / 64 / 48KiB / 1MiB for bz2 (levels 1/5/9), xz (presets
  0/6/9), FORMAT_ALONE, FORMAT_RAW (FILTER_LZMA2);
- 13 CPython-produced base64 vectors (bz2 l1/l9/multi/empty/trunc; xz
  default/CRC32/none/preset0/filters/truncated; alone default/9e; raw lzma2)
  decode byte-identical (len+sha256 pinned);
- bz2 compress hex pins vs CPython at levels 1 and 9;
- 19 error-text cases; constants line.

Ap-conform: values **50/50**, stdlib **48/48**. No corpus case asserted the
old NotImplementedError behavior (grep over tests/ confirms bz2/lzma appear
only in the new gate/probe), so nothing needed updating there; the
t11_stdlib scope_table rows (`bz2`/`lzma` = "cext"/S-05-19) describe
import-face coverage and are unchanged.

`cargo test -p ap-run --release`: 18 suites, all `ok`, 0 failed (after
building the debug `ap-run-eval`/`ap-run-probe` bins the differential suites
spawn — the known stale-binary gotcha: fresh CARGO_TARGET_DIR needs one
debug-bin build before the suites find their fences).

`xtask notices --check` OK (323 deps); `stdlib-provenance --check` OK
(506 files; bz2.py/lzma.py re-declared deviated).

## 4. Bench probe (informational; p11_bz2lzma.py, 48KiB / 1MiB)

GB/s (higher better), best-of-5, M-series host:

| probe | size | CPython | antipythony | ratio |
|---|---|---|---|---|
| bz2.compress(level=1) | 48KiB | 0.0108 | 0.0124 | 1.15x |
| bz2.compress(level=1) | 1MiB | 0.0104 | 0.0118 | 1.13x |
| bz2.decompress(level=1) | 48KiB | 0.2095 | 0.187 | 0.89x |
| bz2.decompress(level=1) | 1MiB | 0.2081 | 0.2068 | 0.99x |
| lzma.compress(xz) | 48KiB | 0.0918 | 3.4695 | 37.8x (divergent encoder, not comparable) |
| lzma.compress(xz) | 1MiB | 0.1245 | 14.44 | 116x (divergent encoder) |
| lzma.decompress(xz) | 48KiB | 0.5596 | 2.0841 | 3.72x |
| lzma.decompress(xz) | 1MiB | 0.626 | 6.5896 | **10.5x** |
| lzma.decompress(alone) | 48KiB | 1.3165 | 0.022 | 0.02x |
| lzma.decompress(alone) | 1MiB | 1.6654 | 0.0225 | 0.01x |

Read honestly: the lzma *compress* rows are a compression-RATIO effect, not
speed — lzma-rs stores literals (output ≈ input size, ~100x less work than
liblzma's LZ match search), so those rows are flagged not-comparable in the
probe docstring. The perf-relevant leg: **bz2.decompress is at parity
(0.89–0.99x CPython, and CPython's arm is libbz2 in C)** — pure-Rust
libbz2-rs-sys holds the bar. lzma.decompress(xz) at 3.7–10.5x CPython looks
like a win but decompresses a much larger lzma-rs stream relative to work —
the apples-to-apples xz-decompress row needs the `xz`-crate swap to mean
anything. alone-decompress is genuinely slower (0.01–0.02x): lzma-rs's LZMA1
decoder lacks the size-guided fast paths liblzma gets from the header's
known unpacked size; a filed lever (LZMA-ALONE-DECODE), low priority (FORMAT_ALONE
is legacy; corpus code overwhelmingly uses xz).

## 5. Honest gaps (filed)

1. Streaming objects: BZ2Compressor/BZ2Decompressor/LZMACompressor/
   LZMADecompressor/BZ2File/LZMAFile raise NotImplementedError lazily (names
   exist so imports/dir keep working). Follow-up shape: incremental
   Compress/Decompress handles over bzip2::mem + lzma-rs stream API.
2. lzma.compress OUTPUT is not byte-identical to liblzma (xz check=NONE;
   alone header dict_size fixed 8MiB by lzma-rs regardless of preset; preset
   maps to lzma-rs defaults). Decode-exactness unaffected. Swap path: `xz`/
   `xz-core` crate (byte-identical-encoder alternative: `xz-native` once
   mature).
3. BCJ/delta filter ids are exposed as constants but reject at use
   ("Invalid or unsupported options") — lzma-rs has no BCJ/delta codecs.
   Rare in corpus code (filed LZMA-BCJ).
4. memlimit accepted but unenforced (liblzma soft limit; documented).
5. lzma.decompress(FORMAT_RAW, FILTER_LZMA1) decodes via lzma-rs raw
   LzmaDecoder — works for CPython-produced LZMA1-raw in the common preset
   dict shape; exotic explicit lc/lp/pb dicts route preset defaults.
6. alone-decompress perf lever (LZMA-ALONE-DECODE above).

## 6. Commits

- 0d6392d deps(pp2-bz2): bzip2 0.6 + lzma-rs 0.3 + THIRD_PARTY_NOTICES.
- 86b6e0e feat(pp2-bz2): real bz2 + lzma one-shot surfaces + gate + probe
  + vendored shim re-faces + DEVIATIONS.toml.
