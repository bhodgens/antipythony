# 02-hexlify-report — P03 (leaf 02-hexlify-simd)

Worktree: leaf-primperf-hex (branch tier1-primperf-hex, from tier1-primperf @
3cb3a55 + merged base64 sibling work 77d62ac/cc47c85). Machine: Mac M2 Ultra
(aarch64, NEON). CPython ref 3.14.7 (/opt/homebrew/bin/python3.14), quiet-ish
window (loadavg 8-10 background, probes best-of-5 in-probe).

## Landed

Commits fdb53a4 (kernels + arms + gate) and 65650cf (bench_report refresh).
Files (hard ownership honored):

- `crates/ap-run/src/simd_encode.rs` — P03 section appended (base64 section
  untouched; `git show --stat` confirms no fmt drift outside our sections)
- `crates/ap-run/src/builtins_modules.rs` — binascii hexlify/b2a_hex +
  unhexlify/a2b_hex call arms only (+ two pub shims `unhex_nibble_pub` /
  `value_bytes_pub` so the kernels reuse the scalar nibble helper without
  duplicating it; `bytearray_state` reused for the unhexlify buffer path)
- `tests/bench/primitive/diff_hex.py` — matrix F added (grouping), 1832 →
  1976 cases
- `tests/bench/primitive/bench_report.json` — P03 rows refreshed (58 rows,
  single-writer this session)

### T1 — differential gate FIRST

diff_hex.py was already GREEN pre-change on this worktree's fresh release
build (1832/0, exit 0). After landing + the matrix-F extension: **1976
cases, 0 mismatches** vs CPython 3.14.7 (smoke mode; `--full` hits the same
worker-leg payload-generation wall documented honestly in 01-base64-report
— per-byte getrandbits in the tree-walker exceeds the 1800s worker timeout;
follow-up for the harness owner, unchanged from the sibling leaf).
diff_b64.py re-verified green after (2132/0) — shared binascii surface
untouched semantically.

### CPython 3.14 baseline (measured, not assumed)

- hexlify: 1.63–1.73 GB/s (1 MiB) — same speed as `bytes.hex` (shared
  hex-to-string core); NOT SIMD-class.
- unhexlify: 3.49 GB/s (1 MiB) — CPython 3.14's `bytes.fromhex` is ~3.35
  GB/s (SIMD-class per the leaf spec note) and binascii.unhexlify rides the
  same fast core. So the decode target was parity-or-better vs a ~3.5 GB/s
  baseline; we exceeded it.

### T2/T3 — NEON kernels (simd_encode.rs P03 section)

- **encode16** (16 bytes → 32 hex chars): nibble-split (`vshrq_n_u8` /
  `vandq 0x0f`) + one 16-byte `vqtbl1q` table + `vzipq_u8` interleave
  (AArch64 ZIP two-half layout: `.0` = first 16 output chars, `.1` =
  second). Threshold 16 bytes.
- **decode16** (32 chars → 16 bytes): clamp-classify nibble (`vqsubq '0'`,
  `vqsubq 'a'-10`, `dig > 9` letter-mask via `vcgtq`, `vbslq` select) →
  then TWO 32-byte `vqtbl2q` gathers (even/odd lane selectors over the
  (n0,n1) pair) + `vshlq 4` + `vorrq`. This gather shape replaced the
  classic Muła shift/combine after the differential gate caught four
  byte-order attempts (vuzp/vzip struct-field conventions on Rust stdarch
  differ from the C intrinsic mental model — the gate's every-length matrix
  did exactly its job).
- **valid_hex_run** (validity scan): NEON-wide, 16 chars per iteration —
  `vcleq(x-'0', 9)` | `vcgeq(x,'a') & vcleq(x,'f')`, all-lanes via
  `vminvq`. This was the decode unlock: the scalar scan was the bottleneck
  (unhexlify 0.94 → 6.5 GB/s when the scan went NEON). Scalar tail + all
  errors stay on the scalar loop.
- `#[cfg(target_arch = "aarch64")]` on all kernels + non-aarch64
  consume-nothing fallbacks (`fallthrough_hex`). No new crate deps.

### Grouping (sep/bytes_per_sep) — implemented CPython-exactly

The previously-ignored args are now full CPython 3.14 semantics, measured
against the real 3.14.7 (the docstring's "negative counts from the right"
is misleading; the gate pinned the actual rule):

- `bps > 0`: separator before byte i iff `i != 0 && (len - i) % bps == 0`
  (right-aligned groups). `hexlify(b'abcde', b':', 2)` = `61:6263:6465`.
- `bps < 0`: separator before byte i iff `i != 0 && i % bps == 0`
  (left-aligned groups). `hexlify(b'abcdef0123456789', b' ', -3)` =
  `616263 646566 303132 333435 363738 39`.
- `bps == 0` / `sep` absent: no grouping (plain NEON bulk path).
- Error surfaces byte-exact: `sep` non-bytes → TypeError
  `object of type 'int' has no len()`; len≠1 bytes/str sep → ValueError
  `sep must be length 1.`; str data → TypeError `a bytes-like object is
  required, not 'str'`; kwargs `sep=`/`bytes_per_sep=` accepted;
  `b2a_hex` identical.
- `unhexlify` takes NO sep (CPython: `takes exactly one argument` / `takes
  no keyword arguments`) — arity surfaces added. Also fixed: bytearray arg
  accepted (bytearray_state buffer), non-ASCII str → ValueError
  `string argument should contain only ASCII characters`, odd-length
  checked BEFORE bad digits.

### Measured (1 MiB, best-of-5 in-probe × 3 runs)

| primitive | before | after | vs CPython 3.14 | gate |
|---|---|---|---|---|
| binascii.hexlify | 0.0336 GB/s | **2.02–2.11 GB/s** | 1.23x | PASS vs "beat CPython" |
| binascii.unhexlify | 0.2353 GB/s | **6.53–6.67 GB/s** | 3.81x | PASS (parity target exceeded) |

48KiB rows: hexlify 1.78 GB/s (1.08x), unhexlify 4.29 GB/s (2.52x) —
dispatch overhead dominates the small size (same shape as the base64 rows).

bench_report.json P03: all four rows now PASS (were FAIL/0.02x-0.13x).
P08/P09 ERROR/TIMEOUT rows pre-existing, untouched. P10/P04-P07 rows
re-measured by the same full run.

### Gates (final tree, all re-run after the last kernel change)

- `diff_hex.py --other target-hex/release/antipythony run` → 1976/0 rc=0
- `diff_b64.py ...` → 2132/0 rc=0
- ap-conform values: 50/50; stdlib: 48/48 (tier 0)
- `cargo test -p ap-run --release` → 208 passed / 0 failed / 1 ignored
  (the known `bugb_*` stale-binary gotcha was pre-empted: symlinked
  `debug/ap-run-eval` → release bin in target-hex before the run; zero
  failures this time)
- loadavg < 10 for the full run_bench.py re-run (single-writer bench_report)

## Honest gaps

1. **hexlify = 2.1 GB/s is 1.23x CPython, NOT the >= 10x dispatch target.**
   Two focused attempts: (a) the intrinsics kernel alone runs ~4.5 GB/s
   standalone in this shape (TBL+vzip on M2 Ultra; upstream hex SOTA is
   ~2x base64's 24 GB/s only on desktop-class AVX-512/AVX2 paths we cannot
   match without runtime dispatch — NEON TBL tops out lower), and (b) the
   probe measures through the tree-walker call arm (~0.5 µs/call fixed
   overhead + Value mint + Gc alloc), which caps any kernel at ~2-3 GB/s
   for 1MiB blocks — the same dispatch-overhead ceiling recorded as the
   base64 encode honest gap. CPython 3.14 hexlify itself is already a
   table-based tight loop at 1.67 GB/s, so a 10x multiple would need ~17
   GB/s through the interpreter arm — not reachable in this leaf's file
   ownership (a tier-1/bulk-call path is the follow-up, mirroring the
   sibling's finding). unhexlify reached 3.8x vs CPython's own SIMD-class
   fromhex core, beating its parity target.
2. **`--full` differential mode unrunnable** (worker-leg payload gen in the
   tree-walker exceeds the harness 1800s timeout) — inherited from the
   sibling leaf's honest limit; the 1976-case smoke matrix (every length
   0..512, invalid char at every position of 4 lengths, SIMD block edges,
   grouping lengths 0..100 × ±1..4) is the recorded gate.
3. **decode16 nibble-merge uses 2× vqtbl2q gathers** instead of the classic
   shift/combine — byte-exactness forced this after the gate rejected 4
   vuzp/vzip-based variants (Rust stdarch struct-field lane conventions).
   Cost is ~1 extra TBL op per 16 output bytes; at 6.5 GB/s vs the 3.5
   GB/s target the margin made it not worth further tuning.
4. **hexlify grouped path is scalar-rearranged** after the NEON bulk
   (matching CPython's own two-pass shape); grouped calls do not exceed
   the plain path's throughput, but grouping is a rare surface (bytes.hex
   default is ungrouped) and the bulk still rides NEON.
