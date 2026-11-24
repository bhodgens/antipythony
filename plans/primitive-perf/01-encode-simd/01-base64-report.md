# 01-base64-report — P01/P02 (leaf 01-base64-simd)

Worktree: leaf-primperf (branch tier1-primperf). Machine: Mac M2 Ultra
(aarch64, NEON baseline). CPython ref 3.14.7 (/opt/homebrew/bin/python3.14).

## Landed

Commit 77d62ac (+ this docs commit), three files (hard ownership honored):

- `crates/ap-run/src/builtins_modules.rs` — b64 arms only
- `crates/ap-run/src/simd_encode.rs` — NEW (NEON kernels)
- `crates/ap-run/src/lib.rs` — one-line `pub mod simd_encode;`

### T1 — API divergences (8/8 fixed, differential-first)

The 8 pre-existing divergences all lived in the `base64`-module arms; the
binascii a2b/b2a arms were already byte-exact. Root cause: the old arms
were hand-rolled approximations instead of the Lib/base64.py wrapper
semantics. Fix: the base64-module arms are now EXACT ports of
Lib/base64.py — thin wrappers over ONE shared `a2b_base64_impl` state
machine extracted verbatim from `call_binascii` (itself a byte-exact
port of binascii.c):

1. `b64encode(altchars=)` — now translates `+/` via the exact 2-byte
   maketrans shape (was ignored).
2. `b64decode(altchars=b'-_')` — translates alt→`+/` before decode
   (was rejecting `-`/`_`).
3. `validate=True` routes to `a2b_base64(strict_mode=True)` →
   "Only base64 data is allowed" (was generic invalid-string text).
4. Bare non-alpha char → "Incorrect padding" per the a2b scan semantics.
5. Unpadded `b'YQ'` → `Incorrect padding` (was wrongly decoding).
6. Interior whitespace skipped without validate= (a2b discards
   non-alphabet bytes like CPython; was rejected).
7. `_bytes_from_decode_data` str/bytes TypeError/ValueError surfaces
   (ASCII-only str decode arg, bytes-like rejection text).
8. `encodebytes` re-chunks at MAXBINSIZE=57 with per-chunk newline
   (76-char line wrap; was a single no-wrap pass).

Gate: `tests/bench/primitive/diff_b64.py --other <bin> run` →
**2132 cases, 0 mismatches, exit 0** (after SIMD landed; also green on
the pre-SIMD intermediate build). diff_hex.py smoke stays green
(1832/0) — shared binascii surface untouched semantically.

HONEST LIMIT: `--full` (10k random lengths to 1 MiB) hits the harness's
own OTHER-LEG 1800s timeout: the worker leg regenerates every payload
with per-byte `random.getrandbits` Python loops inside the tree-walking
interpreter, which alone exceeds 1800s. The smoke matrix (2132 cases:
every length 0..512, invalid byte at every position of 6 lengths,
custom alphabets, validate=, embedded newlines, non-canonical tails,
padding variants) exercises every SIMD boundary (24/48/64/16-char block
edges included) and is the recorded gate. A `--full` run needs the
payload generator hoisted out of the interpreter (pre-computed bytes
passed via env/file) — follow-up for the harness owner, not this leaf.

### T2/T3 — SIMD (PR #162443 port, byte-exact)

`simd_encode.rs` ports Modules/binascii_neon.h structure directly:

- **Encode**: `vld3q_u8` de-interleave → bit-slice → `vqtbl4q_u8` TBL
  over the 64-byte standard table → `vst4q_u8`. 48-byte and 24-byte
  blocks; a ragged multiple-of-3 length finishes with one overlapping
  24/48-byte block (safe: block boundaries stay on 3-byte groups).
  NEON threshold 24 bytes (PR constant). Works for any alphabet by
  construction (TBL), though only the standard table is wired (see
  Decode note).
- **Decode**: Muła/Lemire nibble lookup (`lut_lo/lut_hi` clamp +
  `lut_roll` add, `'/'` via `vceqq` index bump) + `vsliq` packing
  (`pack16`: 16 6-bit values → 12 bytes). 64-char then 16-char blocks;
  stops before the first block containing any rejected character, and
  the scalar loop finds the exact position and owns EVERY error.
  Threshold 32 bytes (PR constant).
- Scalar loops keep: tail, padding, invalid characters, all error
  texts/positions. NEON consumes only complete groups.
- `#[cfg(target_arch = "aarch64")]` on the kernels; non-aarch64 builds
  get consume-nothing fallbacks (scalar-only). No new crate deps
  (`core::arch::aarch64` only).
- The PR's 256-byte custom reverse-table decode path is structurally
  unreachable in our wiring: binascii 3.14 has no `alphabet=` kwarg and
  `b64decode(altchars=)`/`urlsafe_*` translate to `+/` first, so every
  decode that reaches the kernels uses the standard alphabet. The
  custom-table kernel is therefore NOT ported (documented divergence in
  scope, zero behavioral impact); encode-side TBL already handles
  arbitrary alphabets if a future `alphabet=` surface lands.

Rust-specific port notes: tuple structs use `.0/.1/.2/.3` (not
`.val[]`); `uint8x16x4_t` needs full initialization before partial
field assignment (E0381); two bugs found and fixed by the differential
gate on the first SIMD run: (a) wrapper used output-bytes where
`decode_blocks` returns input-chars-consumed (zero-fill + overlong
outputs), (b) gate caught it at lengths 22+ immediately — the gate's
every-length matrix did exactly its job.

### T4 — gates

- `ap-conform values`: **50/50 PASS** (incl. V016).
- `ap-conform stdlib`: **48/48 PASS** (s001–s048, tier 0).
- `cargo test -p ap-run --release`: **208 passed / 0 failed / 1
  ignored** across all suites. (Note: the 2 initial `bugb_*` failures
  were the known stale-binary path issue — suites spawn
  `debug/ap-run-eval`; symlinked into `target-primperf/debug/` from the
  release build, green after. Not a code failure.)
- `run_bench.py` full re-run → bench_report.json refreshed (58 rows,
  single-writer this session). P08/P09 rows remain ERROR/TIMEOUT
  pre-existing (random.Random missing / probe timeout), untouched per
  dispatch instruction.

### Measured (1 MiB, best-of-5 in-probe, quiet window)

| primitive | before | after | vs CPython 3.14 | gate |
|---|---|---|---|---|
| base64.b64encode | 1.26 GB/s | **17.4–18.7 GB/s** | 22.7–23.0x | < 20 target |
| base64.b64decode | 0.45 GB/s | **10.0–10.9 GB/s** | 23.3x | >= 10 PASS |
| binascii.b2a_base64 | 0.77 GB/s | 8.3–10.0 GB/s | 10.8–13.0x | PASS (beat CPython) |
| binascii.a2b_base64 | 0.44 GB/s | 8.0–9.4 GB/s | 18.2–20.1x | PASS (beat CPython) |

48KiB rows: encode 9.0–9.4, decode 7.1–7.6 GB/s (call-dispatch overhead
dominates at the smaller size; same shape as the baseline report).

## Honest gaps

1. **Encode 1 MiB = 17.4–18.7 GB/s vs the >= 20 GB/s gate (missed by
   ~7–12%).** The intrinsics path is byte-exact GREEN, so per the leaf
   decision rule the SWAR fallback was NOT taken. Where the time goes:
   the kernel itself is not the bottleneck — a standalone
   alloc+touch of the 1.4 MiB output Vec costs ~0.5 µs, and the same
   kernel shape through `binascii.b2a_base64` (extra kwargs scan +
   over-allocated buffer, since fixed to delegate through the same
   tight path) lands within noise of the b64encode arm. The residual
   is per-call interpreter dispatch (module-marker call path + Value
   mint per timed call, all inside the tree-walker) — out of this
   leaf's file ownership. Upstream's 25.8 GB/s reference number was
   measured on an M5 Max; M2 Ultra memory/NEON throughput is lower.
   A tier-1 (JIT) call path or bulk-call API would likely clear 20;
   filed as follow-up, not patched here.
2. **`--full` differential matrix unrunnable end-to-end** (worker leg
   payload generation exceeds the harness's own 1800s worker timeout in
   the tree-walking interpreter; see T1). Smoke matrix is the recorded
   gate; a payload-hoisting harness fix is the follow-up.
3. **Custom-alphabet NEON decode kernel not ported** — structurally
   unreachable through the current surface (see T2/T3 note).
4. Decode verdict rows still say FAIL in bench_report.json: the report
   schema's `target_gbs` for b64decode is the PR's 14 GB/s (M5) SOTA
   number, not the leaf's >= 10 gate. Decode clears >= 10; it does not
   clear 14 on this hardware — same hardware-vs-SOTA caveat as encode.
