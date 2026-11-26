# Fix leaf pp2-int: int.to_bytes bulk emit, int(str) small-int fast path, int(str) D&C big parse

Worktree: `/Users/caimlas/.git/worktrees/leaf-pp2-int` (branch `tier1-pp2-int`,
fork of main @ 76e5f4e). Commits 3bed09b / 4adc4ca / 6e0c280. All surfaces
byte-exact vs CPython 3.14 (`/opt/homebrew/bin/python3.14`). No new deps.

## Pre-fix baseline (measured first-hand, fresh release build, P07 slice)

| row | CPython | ours | ratio |
|---|---|---|---|
| int.to_bytes 48KiB | 1.270 GB/s | 0.896 | **0.71x FAIL** |
| int.to_bytes 1MiB | 1.275 GB/s | 1.015 | **0.80x FAIL** |
| int(decimal_str) 48KiB | 0.0096s | 0.0217 | 2.26x PASS (dispatch-bound) |
| int(decimal_str) 1MiB | 0.0015s | 0.0011 | **0.73x FAIL** |
| 4000-digit parse (isolated) | 118.6 µs | ~158 µs | **0.75x** |

## Item 1 — int.to_bytes bulk limb emit (int.rs `to_bytes_exact`)

Root causes found beyond the brief's guess (the per-limb fill loop was NOT
the only problem):
1. Every call — including tiny smis — built `Big::one() << (length*8)` limit
   values (at 1MiB length: a 1MiB BigInt allocation just to range-check) and
   cloned the BigInt twice via `to_big()`.
2. The emit walked `iter_u32_digits().flat_map(to_le_bytes)` — one byte per
   iteration.

Fixes:
- **Smi fast path**: pure-i64 two's-complement math, sign-fill + `to_le_bytes`
  slice copy, sign-bit overflow checks. No Big construction at all.
- **Big arm**: overflow via `bits()` + top-byte inspection (no limit allocs),
  magnitude viewed without clone, then u32-limb array -> one bulk `memcpy`
  (on LE targets the limb array IS the LE byte representation; BE falls back
  to the per-limb clamp loop). Not-limb-aligned `length` handled by clamping
  the copy — limb bytes beyond `length` are proven zero.
- In-helper measurement: **5.16 GB/s** vs CPython 1.28 GB/s.

Result: to_bytes 48KiB **2.58-2.75x** PASS, 1MiB **3.63-3.89x** PASS.

## Item 2 — int(str) small-int i64 fast path (int.rs `parse_base10`)

`cleaned.len() <= 18` digits: pure-u64 accumulate (10^18-1 < i64::MAX, no
overflow possible), `Int::from_i64` normalize. Skips Big entirely.

Isolated parse timing (incl. interpreter dispatch): 18 digits 0.57-0.60 µs,
19 digits 0.63-0.67 µs vs CPython 0.09/0.10 µs. The remaining gap is the
tier-0 call dispatch floor (~190 ns/step territory), not the parse itself —
the string is validated and accumulated in tens of ns. P07 int(decimal_str)
48KiB row stays ~2.3-2.5x PASS; the 1MiB row is a str-repr artifact of the
probe (mostly empty string parse, dispatch-bound both sides).

## Item 3 — int(str) D&C subquadratic big parse

`ndig > 56` (DC_PARSE_THRESHOLD): split into little-endian 9-digit u32
chunks, divide-and-conquer combine `lo + hi * 10^(9*mid)`.

Two correctness traps found and fixed:
1. First cut memoized `(10^9)^(2^level)` powers keyed on
   `floor(log2(mid))` — WRONG for any chunk count that is not a power of
   two (mid=3,5,6,7,...): the hi block's place value is `(10^9)^mid`, not
   the nearest level power. Produced silently wrong values from 57 digits
   up (e.g. `"9"*63` lost its top digits). Caught by the new fix_int.py
   DCS chunk-boundary rows (str round-trip mismatch). Fix: compute
   `10^(9*mid)` directly per split — still subquadratic (each split does
   one Karatsuba/Toom-3 mul by a precomputed-shape power; num-bigint's
   `pow` by repeated squaring is cheap relative to the combine).
2. (en-route) `bit_length()` surface stub (`to_i64().unwrap_or(0)`)
   reported 0 bits for EVERY bigint beyond i64 — pre-existing data bug
   found by the new probe rows' bit_length-based sizing; fixed at
   builtins_modules.rs:18188 with Smi leading-zeros / `magnitude().bits()`.

Result (isolated, `sys.set_int_max_str_digits(1000000)`):

| digits | CPython | ours | ratio |
|---|---|---|---|
| 1000 | 7.6 µs | 8.2 µs | 0.93x (pre-D&C threshold range; native parser comparable) |
| 4000 | 119.2 µs | 31.5 µs | **3.79x** |
| 10000 | 435.0 µs | 132.3 µs | **3.29x** |
| 100000 | 15758 µs | 2611 µs | **6.03x** |

P07 row int(decimal_str) 1MiB: 0.73x FAIL -> **4.33-4.40x PASS**.

(The P07 1MiB decimal string is 1,048,576 digits: our ~3.1ms vs CPython's
~13.6ms-equivalent GB/s view — the harness divides bytes/s, CPython's
libmpdec quadratic path dominates at this size.)

## Gates (all on the final committed tree, fresh binaries)

- (a) `tests/bench/primitive/fix_int.py` EXTENDED: +4145 rows — to_bytes
  big/little × signed × lengths 0..64 on 35 values (smi boundaries, ±2^63/64,
  504-bit pseudo-randoms), 1KiB + 1MiB roundtrips both orders × signed,
  int(str) 16..22-digit boundary + i64 max/min/overflow + underscore + error
  texts, D&C byte-value equality (to_bytes->from_bytes roundtrip) at
  1000/4000/10000 digits + 55/56/57/63/64/65/999-digit chunk-boundary
  shapes, digit-limit raise moved BEFORE the big parses.
  **18653 lines byte-identical vs CPython 3.14** (diff rc=0, exit-code
  contract kept).
- (b) ap-conform rebuilt binaries: **values 50/50**, **stdlib 48/48**.
- (c) `cargo test -p ap-run -p ap-obj --release`: **280 passed / 0 failed**
  (bugb_* debug->release gotcha pre-empted: `target-pp2-int/debug/`
  ap-run-eval + ap-run-probe symlinked to the release bins).
- (d) bench slice P07 (loadavg 6.9-10.9, three runs for stability):
  to_bytes 2.58-2.75x / 3.63-3.89x PASS, decimal_str 2.48-2.55x / 4.33-4.40x
  PASS, from_bytes 1.55-1.67x / 2.38-2.49x PASS; hex 0.74-0.81x FAIL
  (unchanged, NOT in this leaf's scope — filed lever, see honest gaps).

## Honest gaps

- hex()/oct() (0.74-0.81x) untouched — different surface (decimal
  round-trip in the base-digit emitter), same file family but out of the
  leaf brief; the ppfix-int report's 1.4x @4k-digit number was a different
  measurement (net of dispatch, 4000-digit value, not the 1MiB P07 row).
- Small-parse (<=18 digits) absolute time 0.57µs vs CPython 0.09µs: the
  residual is tier-0 call dispatch (~190ns/step floor), not parse work —
  tier-1 territory, filed lever.
- D&C threshold 56 chosen from the isolated sweep (36/45/54-digit shapes
  round-trip fine and D&C wins or ties from ~50 digits); not swept finer —
  between 19 and 56 digits both paths are sub-µs.
- `bit_length()` fix is a driver fix in the int domain but technically a
  new surface change (was silently wrong data); covered by the new probe
  rows + values 50/50, but no dedicated bit_length sweep was added.
- to_bytes BE-target path (per-limb fallback) untested — no BE host
  available; logic reviewed, same output shape as the clamp loop that WAS
  byte-exact-tested.
