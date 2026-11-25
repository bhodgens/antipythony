# fix-random report (leaf ppfix-random)

Branch: `tier1-ppfix-random` @ 770182a (base 4141a86)
Task: implement the `random.Random` class natively so the P08 sort probes
stop crashing; verify p08_sort.py completes and emits bench rows.

## PRNG kind + CPython parity

The pre-existing native random module (builtins_modules.rs) is an EXACT
MT19937 port (init_by_array seeding + genrand_uint32 + genrand_res53
floats). `random.Random` is backed by the SAME `Mt` struct, so the
byte-exactness contract falls out free:

- SAME seed -> SAME sequence across two instances (1000-draw check).
- `Random(seed).random()` == module-level `seed(seed); random()` streams.
- **CPython-exact sequences** (verified against CPython 3.14.0, not just
  self-consistent): `getrandbits` 32/33/63/64 word streams, `randrange`
  (incl. 62-bit widths with rejection sampling), `randint`, `sample`,
  `choice` all match CPython draw-for-draw for the probed seeds.
- Negative seeds use the absolute value (CPython random_seed
  negation->abs), including bigint seeds via `Mt::new_seeded_big`
  (num-bigint to_bytes_le -> init_by_array key words).

## Divergences found and FIXED en route (module-level, pre-existing)

1. `genrand_bits(k > 32)` assembled the words with the trailing shift
   applied to the WHOLE integer; CPython right-shifts the LAST word
   into the high lane (first word full 32 bits low). Every k > 32 draw
   diverged from CPython. Fixed; getrandbits(33/63) now byte-exact.
   (This bug was invisible to the b64/hex gates: they only draw
   getrandbits(8).)
2. `getrandbits(64)` cast the u64 into an i64, returning negative
   values for the top-bit-set half of the space; now returns the
   arbitrary-precision int (CPython-exact).
3. Error texts brought to exact CPython 3.14 wording:
   `empty range in randrange(a, b)`, `empty range for randrange()`
   (any stop <= 0), `zero step for randrange()`,
   `empty range in randint(a, b)`, `Cannot choose from an empty
   sequence`, `Sample larger than population or is negative`,
   `Cannot convert negative int` (getrandbits).

## Known honest divergences (documented, not blocking P08)

- seed(None) is deterministic-seeded (Mt::new_seeded_int(0)) rather
  than urandom — the module-level function had the same behavior; the
  corpus never anchors it. CPython's urandom seeding is deliberately
  NOT reproduced (determinism contract preferred).
- seed types: int (smi + bigint) only; float/str/bytes seeding raise
  the CPython-shaped TypeError text but are not implemented (CPython
  3.14 accepts float/str/bytes/bytearray by converting via sha512 of
  the bytes). No probe or corpus case exercises them.
- randrange with a NEGATIVE step: CPython supports descending ranges
  (randrange(10, 0, -1)); ours raises ValueError. No probe/corpus use.
- getrandbits(k) for k > 64 non-multiple-of-8 raises the CPython
  OverflowError text but values > 64 are clamped to 64 bits rather than
  producing true k-bit bigints (CPython supports arbitrary k).
- `random.Random` methods bound via BoundMethod markers on an Opaque
  instance; `repr` is `<random.Random object>` (CPython shows the
  address). `VERSION` == 3 surface present; `__init_subclass__`/
  subclassing of Random is NOT supported (opaque native type).

## Gates

- (a) Differential probe `tests/bench/primitive/fix_random.py`:
  construction + 1000-draw self-consistency + module-level parity,
  CPython-exact draw anchors, shuffle/sample invariants, error texts.
  GREEN vs CPython 3.14 (self leg + other leg rc=0, pass=true).
- (b) `tests/bench/primitive/probes/p08_sort.py` under the rebuilt
  binary: rc=0, all 6 JSON rows emitted (incl. both 1M-element rows),
  wall ~51s (< 60s standalone, < 120s harness timeout).
- (c) ap-conform: values 50/50, stdlib 48/48 (tier 0) on the leaf tree.
- (d) `cargo test -p ap-run --release`: **208 passed / 0 failed / 1
  ignored** across 18 suites (the 2 initial fix_pass_differential
  failures were the known debug-path hardcode; fixed by the standard
  `target-ppfix-random/debug -> release` symlink).
- (e) bench slice `--only P08` -> SCRATCH
  `/tmp/ppfix-random-bench-p08.json`: rc=0, **6 rows emitted, zero
  ERROR rows** (was 4 ERROR + 2 missing before this leaf). All rows
  are perf-FAIL verdicts (list.sort 0.09-0.12x CPython) — perf is
  OUT of this leaf's scope; the crash fix is what landed.

## Files

- `crates/ap-run/src/builtins_modules.rs` — Random class + MT fixes.
- `crates/ap-run/src/interp.rs` — ctor + type-name surfaces (3 call paths).
- `crates/ap-run/src/builtins.rs` — builtin_type Random arm.
- `tests/bench/primitive/fix_random.py` — differential gate (new).
