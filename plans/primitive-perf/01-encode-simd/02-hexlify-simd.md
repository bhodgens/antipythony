# 02-hexlify-simd — P03

## DISPATCH INSTRUCTION

Any agent: implement in the assigned worktree after 01-base64-simd is
REVIEWED. Do NOT commit. Parent:
plans/primitive-perf/01-encode-simd/orchestrator.md. Estimated ~60K.

Goal: hexlify/unhexlify >= 10x CPython 3.14, byte-exact.

## Algorithm (known SOTA family)

- hexlify encode: same TBL/nibble-split family as base64 encode —
  expand each nibble via a 16-byte table lookup; NEON does 8/16 bytes
  per op. Scalar keeps the tail.
- unhexlify decode: Muła/Lemire-style nibble combine — build two
  16-byte lookup tables for the high/low nibble via clamping
  subtractions, `vqtblq` them, shift+or. CPython 3.14 already uses a
  closely related optimization for `bytes.fromhex` (see
 Objects/bytesobject.c hex_decode via OBJ_GET Hex hooks) — measure
  first; if CPython is already SIMD-class here, the target is parity,
  and the leaf records that honestly.
- Guard `#[cfg(target_arch = "aarch64")]` + scalar fallback, same file
  ownership as leaf 01 (`simd_encode.rs` + the binascii hex arms in
  `builtins_modules.rs`).

## Tasks (TDD)

### T1 — differential gate FIRST
`tests/bench/primitive/diff_hex.py`: every length 0..512, random lengths
to 1 MiB, non-hex char at every position, uppercase/lowercase mix,
`bytes.fromhex` with spaces (if surfaced), exception text comparison vs
python3.14.

### T2/T3 — encode/decode SIMD
Gate order: differential green, then perf (>= 10x CPython hexlify;
decode target depends on the CPython measurement from T1 — parity or
better).

### T4/T5 — gates + report
Corpus + suite green; refresh bench_report.json P03; write
`02-hexlify-report.md`.

## Self-verification checklist

- [ ] diff_hex.py green after
- [ ] hexlify >= 10x CPython (or parity-if-CPython-SIMD, measured)
- [ ] file ownership respected

## Review checklist (orchestrator)

- [ ] CPython baseline measured, not assumed
- [ ] bench_report P03 refreshed
