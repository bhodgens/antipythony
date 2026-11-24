# 01-hash-and-compress — audit P04+P05

## DISPATCH INSTRUCTION

Any agent: audit-only leaf. Do NOT edit implementation code. Do NOT commit. Parent: plans/primitive-perf/02-parity-audit/orchestrator.md.
~50K.

## Tasks

### T1 — read the baseline
P04/P05 rows in `tests/bench/primitive/bench_report.json` +
`baseline-notes.md`.

### T2 — read our implementations
- compress/decompress: where do zlib/bz2/lzma route? (grep
  builtins_modules.rs; expect flate2/bzip2/lzma crate calls or shims —
  determine ACTUAL state, do not assume)
- hashlib: md-5/sha1/sha2 crates in ap-run (Cargo.toml deps) — confirm
  which algorithms are native crate vs pure-python fallback

### T3 — compare
- CPython: zlib → zlib.h (zlib-ng tier), hashlib → OpenSSL EVP
  (hardware SHA extensions on ARM: SHA-2 goes ~2 GB/s+ with the ARMv8
  crypto extension)
- Rust SOTA: flate2 (miniz-oxide or zlib backend), sha2 crate (has
  aarch64 asm feature), md-5 crate
- For each primitive: our ratio, the gap, root cause (wrong crate
  backend? missing hw-accel feature? pure-python fallback?)

### T4 — findings doc
`plans/primitive-perf/02-parity-audit/audits/hash-compress.md` with the
per-primitive verdict table (ALREADY-PARITY / FIXABLE+plan /
NOT-WORTH+reason).

## Self-verification checklist
- [ ] every P04/P05 row explained
- [ ] each FIXABLE has: exact crate/feature flag, est. gain, est. effort
- [ ] cites file:line for our impl sites

## Review checklist (orchestrator)
- [ ] crate claims verified against the actual Cargo.toml deps
- [ ] no implementation edits (git status clean of code files)
