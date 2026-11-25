# Fix leaf ppfix-int: int.to_bytes zero-fill, int.from_bytes >16B truncation, hex/oct/bin O(n)

Worktree: `/Users/caimlas/.git/worktrees/leaf-ppfix-int` (branch `tier1-ppfix-int`,
fork of main @ 4141a86). All fixes byte-exact vs CPython 3.14
(`/opt/homebrew/bin/python3.14`). No new deps (num-bigint/num-traits already
in-tree).

## Pre-fix state (measured first-hand on a fresh release build)

- `(1<<70).to_bytes(16,'big')` → 16 zero bytes (silent zero-fill;
  builtins_modules.rs `n.to_i64().unwrap_or(0)`).
- `int.from_bytes(bytes(range(32)),'big')` → `1736447835066146335` (i128
  accumulator wraps past 16 bytes; `big_or_int`'s Big arm was an admitted
  stub that re-cast to i64 — so even the 16B-range values above i64::MAX
  were wrong).
- `hex(10**4000)` ≈ **2569 µs** ours vs **3.2 µs** CPython (decimal
  round-trip + repeated `div_rem` by 16, O(d²)).
- `to_bytes` overflow NOT detected at all (`256.to_bytes(1,'big')` silently
  succeeded); no CPython OverflowError/ValueError texts anywhere.

## Fix 1 — int.to_bytes (correctness)

- NEW `Int::to_bytes_exact(length, little, signed)` in
  `crates/ap-obj/src/int.rs`: limb-based extraction via
  `iter_u32_digits()` → little-endian byte fill, two's-complement
  inversion for signed negatives, explicit range check against
  `2^(8L)` / `2^(8L-1)` windows before emitting.
- Call site `("pyint", "to_bytes")` in builtins_modules.rs rewritten:
  length type-check (TypeError on non-int), negative-length ValueError
  (`length argument must be non-negative`), byteorder validation
  (`byteorder must be either 'little' or 'big'`), `signed` positional or
  kwarg, OverflowError with CPython-exact text from the helper.
- CPython 3.14 message ground truth (probe-verified): the ONLY two texts
  are `can't convert negative int to unsigned` (unsigned && negative) and
  `int too big to convert` (everything else — signed direction does NOT
  flip the text; `-(2**63).to_bytes(7, signed=True)` says "int too big").

## Fix 2 — int.from_bytes (correctness)

- NEW `Int::from_bytes_exact(bytes, little, signed)` in ap-obj/int.rs:
  zero-strip, pad to u32 limb boundary, limb-accumulate `mag=(mag<<32)|d`,
  two's-complement correction `mag - 2^(8n)` when the sign bit is set and
  signed=True, `Int::from_big` normalization.
- Call site `("pyint", "from_bytes")` rewritten to call the helper after
  the existing bytes-likes data resolution; byteorder validated with the
  CPython ValueError text.
- `big_or_int` stub FIXED (builtins_modules.rs): the Big arm now
  `Int::from_big(BigInt::from(v))` instead of re-casting to i64 (this
  helper also serves Fraction numerator/denominator at :19387).

## Fix 3 — hex()/oct()/bin() O(n) (perf)

- `builtin_hex/oct/bin` (builtins.rs) now share `int_to_base_digits(i,
  base)`: magnitude via `Int::Big(b).magnitude()` (BigUint, no clone of
  sign machinery), `to_u32_digits()` materialized ONCE, then per-digit
  random access `digit d = bits [d*bits, d*bits+bits)` with a straddling
  path for octal (bits=3 does not divide 32). No decimal round-trip, no
  division — CPython `long_format_binary` shape.
- Two implementation traps found and fixed en route (both would have
  silently kept the old behavior/perf):
  1. `mag.iter_u32_digits().nth(k)` per step = O(n²) re-walk (forward-only
     iterator). First cut used it and even a WORKING build showed ~5900×
     no-op; replaced by per-digit limb indexing.
  2. `b.magnitude()` returns `BigUint`, not `BigInt` — first cut didn't
     compile, and one intermediate "successful" build was STALE (old
     binary mtime; the AGENTS.md stale-fingerprint gotcha). All timing
     claims below are re-verified against a binary whose mtime postdates
     the last source edit.
- Legacy `bigint_to_base` retained `#[allow(dead_code)]` (no remaining
  callers; removed risk for any future non-power-of-two user).

## Measured results (fresh binary, in-process perf_counter, net of dispatch)

| probe | CPython 3.14 | ours pre | ours post |
|---|---|---|---|
| hex(10^1000) | 0.83 µs | ~200 µs | **2.5 µs total (1.4 net)** |
| hex(10^4000) | 3.24 µs | 2569 µs | **5.7 µs total (4.6 net)** |
| hex(2^132895) | — (46.9 µs at 2^50000) | 241,980 µs | **45 µs (~5400×)** |
| bin(2^50000) | 46.9 µs | 135,942 µs | **61 µs (~2200×)** |

## Gates (all re-run on the final tree, fresh binaries)

- (a) DIFFERENTIAL `tests/bench/primitive/fix_int.py`: to_bytes/from_bytes
  roundtrips (lengths 0..40 × big/little × signed ±, values incl.
  0/±1/255/±2^63/2^64/±2^70/±2^128 + 40 seeded pseudo-random bigints),
  from_bytes 1..64-byte sweep, audit repros V1-V9, error-text cases E1-E11
  (exact CPython strings), hex/oct/bin of 0/1/-1/255/2^64/10^100/negative
  variants → **9508 lines, byte-identical vs CPython 3.14** (diff rc=0).
  Roundtrip assert `int.from_bytes(b)==v` holds for every emitted buffer.
- (b) ap-conform with the rebuilt binaries: **values 50/50**, **stdlib
  48/48** (tier 0).
- (c) `cargo test -p ap-run --release`: **208 passed / 0 failed / 1
  ignored** (18 suites; bugb_* stale-binary gotcha pre-empted by
  symlinking debug/ap-run-eval + debug/antipythony → release bins).
- Big-output byte-diff: hex/oct/bin of 10^4000 and negatives byte-identical
  vs `python3.14 -X int_max_str_digits=0`.

## Honest gaps

- Error texts probed cover the surface in fix_int.py; exotic arg shapes
  (`to_bytes(1, b'big')` bytes-literal byteorder, `from_bytes(str, ...)`)
  not exhaustively swept (pre-existing parsing shared with the old path).
- Our `int.from_bytes(list_of_ints)` arm filters `to_i64() as u8` — inputs
  >255 silently mask instead of raising ValueError like CPython
  (pre-existing, surface-level, unchanged here; noted for a follow-up).
- format-spec 'x'/'X'/'o'/'b' (format.rs `to_u128_or_panic`) still
  zero-fills beyond u128 — separate surface, NOT touched by this leaf
  (filed for the follow-up queue).
- `hex(10^4000)` net 4.6µs vs CPython 3.2µs: residual is the O(n) emit +
  PyString alloc; not chased further (ratio 1.4× at 4k digits, converged).
