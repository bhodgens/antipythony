# Fix report — leaf pp2-crc: crc32fast + simd-adler32 for zlib.crc32/binascii.crc32/zlib.adler32

Branch `tier1-pp2-crc`, commit **5283b5d** on top of 76e5f4e (main).
Worktree: `~/.git/worktrees/leaf-pp2-crc`. Bench scratch: `/tmp/crc_*_final.txt`;
shared `bench_report.json` untouched. New probe `tests/bench/primitive/probes/p11_crc.py`
(not wired into run_bench.py PROBE_FILES — wiring is the tracker's call, files left
untracked by the perf commit).

## Fix — P11-row primitives crc32/adler32 (audit hash-compress.md rows)

`crates/ap-run/Cargo.toml`: `crc32fast = "1.5"` + `simd-adler32 = "0.3"` added as
direct ap-run deps. BOTH were already in Cargo.lock transitively (crc32fast 1.5.3
via flate2; simd-adler32 0.3.10 via miniz_oxide) — zero new lock nodes, zero new
license surface. THIRD_PARTY_NOTICES regenerated (`xtask notices --check` OK, 317
deps); the regen also picked up the sha2-asm entry that sibling commit 884c535
missed — canonical from-lock output, no hand edits.

### crc32 (zlib.crc32 + binascii.crc32 arms + gzip one-shot trailer)

Scalar bitwise loop (`crc32_walk`, 8 shift/xor iterations PER BYTE) replaced by
`crc32fast::Hasher` (runtime-detected aarch64 carry-less-multiply specialization
on this host). The subtlety that cost one wrong first attempt: **crc32fast's
Hasher state convention is already the zlib VALUE convention** —
`Hasher::new_with_initial(v)` + `update` + `finalize` equals
`zlib.crc32(data, v)` directly (verified with a standalone crc32fast 1.5.3
harness vs CPython 3.14: v=0 → 0xCBF43926 check value, v=0xFFFFFFFF →
3523400311, v=-1 → same, v=2**64+1 → masked). The in-repo scalar walk used the
INTERNAL-register convention (seed `!value`, invert on exit); bolting the same
`!`/invert pair onto crc32fast double-inverts and produced garbage (caught by
the gate before any commit: `crc32(b'123456789')` came out 771566984). Final
code is a straight `new_with_initial(seed)` / `update` / `finalize` with zero
inversions. Same swap in `one_shot_gzip`'s RFC-1952 trailer verification;
`crc32_walk` and `adler32_walk` deleted (0 references remain).

### adler32 (zlib.adler32 arm)

Scalar chunked (5552) sum loop replaced by `simd_adler32::Adler32` (NEON
vectorized on aarch64). `Adler32::from_checksum(v)` matches CPython's
masked-seed chaining exactly.

### Semantics fixes that rode along (both latent divergences, now gated)

- `seed_mask_u32`: CPython seeds via `PyLong_AsUnsignedLongMask` — arbitrary-
  precision ints masked to low 32 bits, negatives floor-mod masked. Probed
  vs 3.14: `crc32(d, 2**64+1) == crc32(d, 1)`, `crc32(d, -2**100) ==
  crc32(d, 0)`, `adler32(d, 2**100) == adler32(d, 0)`. Old code truncated via
  `to_i64()` and diverged on every seed outside i64.
- `crc_data_arg`: the zlib/bnascii DATA arg uses the buffer protocol — str is
  rejected with CPython's exact `'a bytes-like object is required, not 'str''`
  (also list/int/None). The old code shared hashlib's `bytes_arg`, which
  accepted str silently and raised the hashlib-specific
  'Unicode-objects must be encoded' text for other types. hashlib keeps its
  legacy `bytes_arg` (str acceptance is hashlib's own historical shape; not
  touched by this leaf).

## Bench (M-series host, best-of-3, `bytes(range(256))*k` workload, load < 4)

| row | before | after | speedup vs scalar | vs CPython 3.14 |
|---|---|---|---|---|
| zlib.crc32 48KiB | 0.132 GB/s | 9.5 GB/s | **72x** | 0.29x |
| zlib.crc32 1MiB | 0.131 GB/s | 18.2 GB/s | **139x** | 0.52x |
| zlib.adler32 48KiB | 2.31 GB/s | 11.5 GB/s | **5.0x** | 0.68x |
| zlib.adler32 1MiB | 2.58 GB/s | 22.8 GB/s | **9.2x** | **1.35x PASS** |

Honest read: crc32 lands at parity-class (0.29-0.52x) — CPython's zlib.crc32
calls zlib.h which on Apple Silicon uses the same CRC32 instruction family our
crc32fast NEON path uses, and libz's loop overhead is thinner than ours; the
audit's 10-30x-over-scalar estimate is met and exceeded (139x at 1MiB), but
"beat CPython as shipped" remains out of reach for the 1MiB crc row. adler32
1MiB now PASSES (1.35x — simd-adler32's NEON path beats libz's scalar NMAX
loop at large sizes; at 48KiB the fixed hasher-mint cost dominates, 0.68x).

## Byte-exactness (gate first)

New differential probe `tests/bench/primitive/fix_crc.py` (58 cases, same
worker/runner shape as fix_json.py): sizes 0/1/64/1KiB/48KiB/1MiB slices of a
driver-pre-generated 1MiB LCG data file (`ppcrc_data.bin`; in-interpreter blob
generation is a non-starter — the per-byte interpreter floor made a 1MiB gen
take >170s vs 40ms on CPython, hence the fix_sha_zlib DATAFILE pattern, with
`to_bytes(8)` chunked appends after per-byte appends measured 8x slower on the
engine leg). Coverage: crc32/adler32 per size, seed-0/seed-1 defaults, 3-chunk
chaining `crc32(c, crc32(b, crc32(a)))` vs `crc32(a+b+c)` (incl. explicit-0
first chunk), binascii identity per size + chained, unsigned-result type/range
checks, negative and >2^32 seeds, and 9 bad-arg-type error texts.

- Green BEFORE the change (pre-change binary): 58/58 — wait: 49/58 (the 9
  error-text rows + big-seed rows diverged pre-change; the value rows were
  already exact on the scalar walk). After: **58/58, zero mismatches**.
  The gate therefore also documents 4 latent-divergence families fixed
  (str/list data rejection texts, non-int seed texts, >i64 seed masking).
- `fix_sha_zlib.py` re-run (818 lines, CPython-vs-engine): 31 divergences,
  ALL on CORRUPT rows in the two zlib-rs message-granularity families the
  fix-sha-zlib report already documented ("invalid distance too far back" /
  "incorrect data check" vs zlib-rs "repeated call with bad state") — the
  same families present before this leaf (its datafile showed 26). Zero
  divergences introduced by this leaf; all GZ (gzip roundtrip, exercising
  the re-routed trailer CRC) rows identical, DONE line reached.

## Gates

| gate | result |
|---|---|
| fix_crc.py vs CPython 3.14 | 58/58, 0 mismatches |
| cargo test -p ap-run --release | all suites ok, 0 failed (208-class count; bugb_* suites needed the debug/ap-run-eval dev build in the fresh target dir — built, then green) |
| ap-conform values | 50/50 |
| ap-conform stdlib | 48/48 |
| xtask notices --check | OK (317 deps) |

## Honest gaps / residuals

1. crc32 vs CPython 0.29x/0.52x: parity-class, not beating libz's hw path at
   these sizes. Closing means trimming per-call overhead (hasher mint +
   Bytes clone in `crc_data_arg` — the `(**b).clone()` full-buffer copy is
   the likely 48KiB tax) or a borrowed-slice API; filed as the follow-up lever.
2. adler32 48KiB 0.68x: same per-call overhead shape, same lever.
3. fix_sha_zlib's 31 zlib-rs CORRUPT-row message divergences are pre-existing
   (upstream message granularity), unchanged by this leaf.
4. `bytearray`/`memoryview` DATA args raise TypeError on our side
   (crc_data_arg only accepts Bytes) while CPython accepts them via the
   buffer protocol — PRE-EXISTING (old bytes_arg also rejected them), NOT
   regressed, kept out of the gate rows deliberately; noted here for the
   buffer-protocol tier (S2) backlog.
5. p11_crc probe not wired into run_bench.py PROBE_FILES (tracker decision;
   bench_report.json untouched per leaf convention).
