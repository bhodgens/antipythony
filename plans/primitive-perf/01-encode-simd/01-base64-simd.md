# 01-base64-simd — P01/P02

## DISPATCH INSTRUCTION

Any agent: implement in the assigned worktree. Do NOT commit. Parent:
plans/primitive-perf/01-encode-simd/orchestrator.md. Estimated ~90K.

Goal: b64encode >= 24 GB/s, b64decode >= 14 GB/s on this M2 Ultra
(upstream SOTA per CPython PR #162443), byte-exact vs CPython 3.14.

## Upstream algorithm (the spec — port THIS, do not invent)

Source: python/cpython PR #162443 (gh-124951, "Use NEON for base64 in
binascii on AArch64"). Read the PR diff on GitHub first
(https://github.com/python/cpython/pull/162443/files) and treat its
structure as the spec:

- NEON consumes ONLY complete groups: encode = 3-byte groups (or a
  12-byte/16-byte supergroup unrolled), decode = 4-char groups.
- Scalar loops keep: tail, padding (`=`), invalid characters, ALL error
  reporting and exact positions. NEON decode stops BEFORE any 16/64-char
  block containing a rejected character; scalar finds the exact spot.
- Encode with any alphabet: `TBL` instruction over the 64-byte table
  (NEON TBL works for arbitrary alphabets — keeps urlsafe + custom
  alphabets correct).
- Decode standard alphabet: Muła/Lemire nibble-lookup technique
  (-'A' + clamp + pack via vqtblq / vshrn). Decode custom alphabets:
  256-byte reverse table scalar (unchanged).
- Below 24 bytes encode / 32 bytes decode: skip SIMD entirely (short
  inputs unchanged).
- PR leaves SSSE3/AVX2 out (needs runtime dispatch); we target aarch64
  NEON only — this host is M2 Ultra. Gate the code with
  `#[cfg(target_arch = "aarch64")]`, scalar fallback elsewhere.

## Rust implementation notes

- `crates/ap-run/src/simd_encode.rs` (new): the SIMD kernels +
  `#[cfg]` fallbacks. Use `core::arch::aarch64::*` intrinsics directly
  (std has them; no new crate deps — master dependency posture).
- Consider chunked u64 SWAR first if intrinsics stall: the
  "process 12 bytes with three u32 loads + lookup tables" SWAR variant
  historically hits 8-15 GB/s on this class of hardware and is much
  simpler to make byte-exact. DECISION RULE: implement intrinsics path;
  if the byte-exact gate stays red after 2 attempts, land the SWAR
  version with its measured number and record the delta vs 24 GB/s as
  the leaf's honest gap.
- Wire points: `b64_encode` / `b64_decode` in
  `crates/ap-run/src/builtins_modules.rs` (~line 12816) keep their
  signatures; the SIMD path runs for the standard alphabet on both
  `base64.b64*` and `binascii.b2a/a2b_base64` call arms.

## Tasks (TDD)

### T1 — differential gate FIRST (RED)
Write `tests/bench/primitive/diff_b64.py`: deterministic random matrix —
every length 0..512, 10k random lengths to 1 MiB, invalid byte at every
position of a 64-byte string, custom-alphabet a2b/b2a, validate= kwarg,
newlines embedded. Compares antipythony vs python3.14 (bytes AND
exception type+text). Runs green BEFORE optimization (proves the scalar
baseline is exact), must STAY green after.

### T2 — encode SIMD
Implement encode (TBL approach). Gate: T1 green. Perf gate: p01 probe
>= 20 GB/s encode at 1 MiB.

### T3 — decode SIMD
Implement decode (nibble lookup, standard alphabet; scalar reverse-table
for custom). Gate: T1 green including invalid-character positions. Perf
gate: p01 probe >= 10 GB/s decode at 1 MiB.

### T4 — corpus + suite gates
- ap-conform values + stdlib slices green (48/48 values etc.)
- `cargo test -p ap-run --release` green
- re-run full `run_bench.py` → refresh bench_report.json rows P01/P02

### T5 — report
Append measured numbers + the PR port notes to
`plans/primitive-perf/01-encode-simd/01-base64-report.md`.

## Self-verification checklist

- [ ] diff_b64.py green after (proves byte-exactness preserved)
- [ ] V016 + base64 corpus cases green
- [ ] encode >= 20 GB/s, decode >= 10 GB/s at 1 MiB (or honest SWAR gap recorded)
- [ ] no changes outside the declared file ownership
- [ ] `#[cfg(target_arch = "aarch64")]` guards; scalar fallback compiles

## Review checklist (orchestrator)

- [ ] algorithm matches the PR structure (scalar owns ALL error paths)
- [ ] differential matrix actually exercised the SIMD boundary
  (check a length near 24/32 and an invalid char inside a would-be
  SIMD block)
- [ ] bench_report.json P01/P02 rows refreshed with real numbers
