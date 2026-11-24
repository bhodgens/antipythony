# Audit: P06 str methods + P07 bytes/int conversions (02-str-and-bytes)

Audit-only leaf. Worktree: `/Users/caimlas/.git/worktrees/leaf-primperf`
(branch `tier1-primperf`). All `file:line` cites verified by reading the
lines on 2026-11-24. **Measured ratio column: PENDING baseline (W2)** —
`tests/bench/primitive/bench_report.json` does not exist yet (sibling leaf
00-harness is producing it); CPython-only absolute samples below were taken
first-hand on `/opt/homebrew/bin/python3.14` (aarch64 macOS) for gap context.

Every P06/P07 row is explained. No implementation code was touched.

---

## 1. Our implementation state (file:line)

### 1.1 String repr (ap-obj/src/value.rs)

- `PyString` = enum `StrRepr { Ascii(Box<str>), Unicode(Box<str>) }` +
  cached code-point len + cached hash — value.rs:213-231. `as_str()`
  returns the UTF-8 `&str` either way (value.rs:248-252).
- Consequence: **all str method ops are byte-slice ops on UTF-8**, mostly
  delegating to Rust `std` (`str::find/replace/split/trim/to_uppercase`).
  The `Ascii` discriminant is NOT exploited by the method arms (no
  `make_ascii_uppercase` fast path) — minor, see §3 upper verdict.

### 1.2 str methods

| method | site | algorithm |
|---|---|---|
| `upper`/`lower` | ap-run/src/methods.rs:1517-1518 (`call_method`); also pybytes.upper at builtins_modules.rs:18588-18610 (ASCII table walk) | `str::to_uppercase()` — std full Unicode tables, per-char |
| `join` | methods.rs:1519-1563 | borrowed `Vec<&str>` collect + one pre-sized `String::with_capacity` + push_str loop (memperf D9 rework). Optimal shape. |
| `find`/`rfind`/`index`/`rindex` | builtins_modules.rs:17196-17235 (pystr) | `str::find`/`rfind` (std **two-way** Crochemore–Perrin) on a sliced `[start:end]` window with CPython bound semantics |
| `count` | builtins_modules.rs:17245-17269 | `str::matches(sub).count()` — std two-way |
| `replace` | builtins_modules.rs:17175-17186 | `str::replace` — std two-way search + growable String |
| `split`/`rsplit` | builtins_modules.rs:17061-17174 | `str::split`/`splitn`/`rsplit` (std two-way) for explicit sep; hand loop w/ `trim_start` + `find(char::is_whitespace)` for sep=None; per-part `String::from` |
| `startswith`/`endswith` | builtins_modules.rs:16948-17006 | std `starts_with`/`ends_with`, tuple-of-prefixes `.any()` |
| `strip` family | builtins_modules.rs:17007-17060 | std `trim_matches` w/ 256-entry ASCII bitmap + small non-ASCII vec (memperf D6) |
| `splitlines` | builtins_modules.rs:17280-17329 | single `char_indices` pass; bytes variant :17339-17376 (byte walk) |

Surface list for the pystr arm: builtins_modules.rs:15876-15902.

### 1.3 bytes methods (P07-adjacent)

- `bytes.find/rfind/index/rindex`: builtins_modules.rs:18457-18482 —
  **`b.windows(sub.len()).position(|w| w == sub)`** (:18469-18471) =
  naive O(n·m) sliding-window compare. No start/end bound args here
  (surface-only limitation, correctness note not perf).
- `bytes.replace`: builtins_modules.rs:18413-18446 — loop of
  `windows(old.len()).position(...)` (:18433), O(n·m) per occurrence scan.
- `bytes.split/rsplit`: builtins_modules.rs:18235-18300 — same naive
  `windows(sp.len()).position` (:18267) / `.rposition` (:18287).
- `bytes.count`: builtins_modules.rs:18493-18504 — `windows().filter().count()` naive.
- `bytes.upper/lower`: builtins_modules.rs:18588-18610 — `make_ascii_*` table
  walk (correct: bytes upper is ASCII-only per CPython).

### 1.4 int(str) decimal (ap-obj/src/int.rs) — small vs big

- Entry: builtins.rs:851-885 `parse_int_str` (digit-limit check vs
  `INT_STR_DIGIT_LIMIT`, then dispatch base 10 / other).
- `Int::parse_base10` (int.rs:476-510): trim, sign strip, underscore
  validation (allocates a `cleaned: String` at :489), then
  **`cleaned.parse::<Big>()`** (:503) — i.e. **num-bigint `BigInt` parse for
  EVERY input, including 3-digit ints**. `Int::from_big` (:506, int.rs:122-128)
  normalizes down to `Int::Smi` afterwards.
  → **There is NO small-int fast path**: `int("123")` allocates a heap
  BigInt, parses it through num-bigint's radix machinery, then converts
  back. CPython's small path is a straight strtol-class loop
  (`Objects/longobject.c:long_from_string_base`), zero heap for ≤ 2^30-digit
  limbs. This is the exact small-vs-big distinction the review checklist
  asks for, and it is a real regression vs CPython on the hot small path.
- `Int::parse_base` (int.rs:514-554): `Big::parse_bytes(bytes, base)` (:547) —
  same num-bigint machinery for all bases.
- `str(int)`/repr: `to_string_raw` (int.rs:463-468) = `Big::to_string()` —
  num-bigint's decimal conversion, schoolbook divide-by-10^k per chunk.

### 1.5 from_bytes / to_bytes / hex

- `int.to_bytes`: builtins_modules.rs:16895-16928. **Coerces via
  `n.to_i64().unwrap_or(0)`** (:16902-16903) — a bigint ≥ 2^63 silently
  yields `0` bytes. **Correctness bug, not just perf.** Byte loop is fine
  for the i64 case.
- `int.from_bytes`: builtins_modules.rs:19189-19247. Accumulates into an
  **`i128`** (:19229-19245, sign handling :19243-19245) — silently wrong for
  > 16-byte data — and returns via `big_or_int` (builtins_modules.rs:19661-19669)
  whose "big path" **is a stub that casts to i64** (comment at :19665
  admits it). **Correctness bug:** `int.from_bytes((10**200).to_bytes(84,'big'))`
  truncates. Perf for ≤ 8 bytes is fine (one multiply-free shift loop).
- `hex()`: builtins.rs:2600-2613 `builtin_hex` → decimal `to_string_raw()`
  → try `u128` → else `bigint_to_base` (builtins.rs:2659-2677): parse the
  **decimal string** back into a `BigUint`, then repeated
  `div_rem(&n, &b)` by 16 (:2669-2674) — **O(d²) word operations** in the
  digit count. Same helper serves `oct()` (builtins.rs:2615+) and `bin()`.
  `bytes.hex` (builtins_modules.rs:17443+) is a separate, fine, byte→nibble
  table path.

---

## 2. CPython reference (algorithm knowledge; confidence noted)

| primitive | CPython algorithm | site | confidence |
|---|---|---|---|
| str.find/index/count | `stringlib fastsearch`: memchr-class skip for 1-char needles; multi-char = Bloom-filter + **Crochemore–Perrin two-way** (since 3.10; adaptive) | Objects/stringlib/fastsearch.c, `STRINGLIB(find)` | high |
| str.replace | fastsearch-based scan (`stringlib_replace` in Objects/unicodeobject.c): memchr-accelerated first-char skip + pre-sized result buffer | unicodeobject.c | high (pre-3.14 layout; algorithm stable) |
| str.split | fastsearch scan, list pre-sized, zero-copy views internally | unicodeobject.c `unicode_split` | high |
| str.upper | `upper_ucs4`: per-char Unicode case tables, ASCII fast branch | unicodeobject.c | high |
| int(str) small | `long_from_string_base`: word-at-a-time base-10^k accumulation into PyLong limbs, **no heap beyond the result** | Objects/longobject.c | high |
| int(str) big | `_pylong.int_from_string` above a decimal-digit threshold (~450 digits; confidence medium on the exact constant): builds a **`decimal.Decimal`** (C `_decimal`, libmpdec) and converts — subquadratic O(n·log n)-ish | Lib/_pylong.py | medium-high |
| str(int) big | `_pylong.int_to_decimal_string` via libmpdec | Lib/_pylong.py | medium-high |
| hex(int) | `long_format_binary`: O(n) bit-pluck over 30-bit limbs, **no division at all** | Objects/longobject.c | high |

CPython 3.14 first-hand samples on this host (for gap sizing; our ratios
wait on W2):

```
int small ("12345")      0.041 µs        hex 4k-digits    3.0 µs
int(str) 4k digits     116.7 µs          str(int) 4k    196.3 µs
to_bytes 8B              0.038 µs        from_bytes 84B  0.18 µs
str.find 100KB           ~9.1 µs         replace 100KB  ~431 µs (both alloc)
split 100KB            ~232 µs           join 50k×20B   ~485 µs
upper 100KB             ~31 µs
```

## 3. Rust SOTA + verdicts

### P06 — str methods

| primitive | our impl | CPython | Rust SOTA | Measured ratio | verdict |
|---|---|---|---|---|---|
| str.find/index/count | std two-way (17196-17235, 17245-17269) | memchr + two-way | **memchr crate**: SIMD (SSE2/AVX2/NEON) find/memrchr/substring search, typically 2-10× std's scalar two-way on large hay | PENDING baseline (W2) | **FIXABLE** — swap `str::find` for `memchr::arch`-backed search behind a helper. Est. gain 2-5× on ≥ 100KB workloads (near-zero on small strings). Effort: trivial per site, **gated on the memchr dep (user decision)**. |
| str.replace | std two-way + growable String (17175-17186) | memchr skip + pre-sized buffer | memchr crate + exact-size precount | PENDING baseline (W2) | **FIXABLE (low)** — precount occurrences for exact `with_capacity` (kills realloc), optional memchr scan. Est. gain 1.2-2×. Effort: small. |
| str.split/rsplit | std split + per-part String (17061-17174) | same class (fastsearch scan) | std is already two-way; memchr adds little for multi-char sep | PENDING baseline (W2) | **ALREADY-PARITY** — std two-way ≈ CPython's scan class; per-part `String` copies mirror CPython's list-of-str materialization. No algorithm gap. |
| str.join | pre-sized single-pass borrowed copy (methods.rs:1519-1563) | pre-sized single-pass | already optimal shape | PENDING baseline (W2) | **ALREADY-PARITY** — the D9 rework already does the exact-reservation trick; any residual ratio is allocation/GC noise, not algorithm. |
| str.upper/lower | std per-char Unicode tables (methods.rs:1517-1518) | per-char Unicode tables w/ ASCII fast branch | std tables ≈ same class; cheap ASCII win available: `is_ascii()` → `to_ascii_uppercase` (byte-wise, table-free) | PENDING baseline (W2) | **ALREADY-PARITY** (upgrade path exists): for ASCII-dominated workloads a 1-line `is_ascii` fast path gains ~2× (confidence medium). Only worth bundling with another fix wave — not its own leaf. |

### P07 — bytes/int conversions

| primitive | our impl | CPython | Rust SOTA | Measured ratio | verdict |
|---|---|---|---|---|---|
| bytes.find/count | naive `windows().position` O(n·m) (18457-18482, 18493-18504) | memchr-accelerated fastsearch | **memchr crate** `ByteSlice::find/memchr/memrchr` — SIMD | PENDING baseline (W2) | **FIXABLE** — biggest byte-side win: naive→SIMD is commonly 10×+ on large buffers. Effort: trivial per site. **memchr dep = user decision.** |
| bytes.replace | naive windows loop (18413-18446) | memchr skip + pre-sized | memchr + precount | PENDING baseline (W2) | **FIXABLE** — same fix family as bytes.find. |
| bytes.split/rsplit | naive windows loop (18235-18300) | fastsearch scan | memchr | PENDING baseline (W2) | **FIXABLE** — same fix family. |
| int(str) small | num-bigint parse for all sizes (int.rs:503) | strtol-class limb loop, no heap | i64 fast parse (`i64::from_str`-class) then promote | PENDING baseline (W2) | **FIXABLE** — add a `cleaned.len() <= 18` i64-parse fast path before the Big parse; keeps byte-exact semantics (from_big still normalizes). Est. gain ~5-10× on the hot small path (CPython 41ns vs our alloc+parse). Effort: tiny, **no new dep**. |
| int(str) big | num-bigint `from_str_radix` = schoolbook multiply-by-10^k chunks, O(n²)-flavored | _pylong threshold + libmpdec subquadratic | subquadratic decconv (divide-and-conquer, or fearless_simd vectorized digit folding per leaf spec); heavyweight alt = `rug`/GMP (NOT proposed) | PENDING baseline (W2) | **FIXABLE** — only bites ≥ ~1000 digits. Plan: split digits in halves, recursive d&c (parse half → ×10^(len/2) + half) using existing num-bigint ops; est. gain O(n²)→O(n^1.58) ≈ 5-20× at 4k-10k digits. Effort: moderate (~1 leaf). Do it only if W2 shows a bad ratio at the bench's digit size. |
| int.to_bytes | i64-only, bigint silently zeroes (16895-16928) | limb-extract, any size | `to_u64_digits`/`to_bytes_le` from num-bigint (already our dep) | PENDING baseline (W2) | **FIXABLE** — correctness first: bigint arm must extract limbs (num-bigint `to_u32_digits` already available) instead of `unwrap_or(0)`. Perf then matches CPython trivially. Effort: small. **This one is a correctness fix, priority over perf.** |
| int.from_bytes | i128 accumulator + stub big path (19189-19247, 19661-19669) | limb accumulate, any size | `BigInt::from_bytes_be/le` — **num-bigint has this exact constructor already** | PENDING baseline (W2) | **FIXABLE** — correctness first: > 16 bytes silently truncates today (big_or_int stub). Replace the i128 loop with `Big::from_bytes_*` + smi normalization. Effort: small, no new dep. |
| hex()/oct()/bin() | decimal→string→BigUint→repeated div_rem, O(d²) (builtins.rs:2600-2677) | O(n) limb bit-pluck, no division | num-bigint radix conversion via binary limbs (d&c or `to_radix_le`-style) | PENDING baseline (W2) | **FIXABLE** — CPython does hex(10^4000) in 3 µs; our path re-parses the decimal string AND divides per output digit. Plan: hex/oct/bin = limb-shift extract from the stored `Big` (never round-trip through decimal). Est. gain order-of-magnitude at thousands of digits. Effort: small-moderate, no new dep. |

### Dependency posture

- **memchr crate is a NEW dependency** and the enabler for five FIXABLE rows
  above (str.find, str.replace optional, bytes.find/replace/split/count).
  Per master dependency posture this is **explicitly a USER DECISION — it
  does not land by leaf authority.** Scoped alternative without a dep:
  hand-rolled SSE2/NEON 16-byte-swap compare loops behind
  `#[cfg(target_arch)]` with scalar fallback (orchestrator Coding
  Conventions already require the aarch64 guard shape) — ~80 lines, no
  supply chain, ~70% of the win. That is the recommended fallback if the
  user declines the dep.
- aho-corasick: **not proposed** — no multi-pattern primitive in P06/P07.
- rug/GMP: **not proposed** (massive dep, licensing, and num-bigint +
  d&c decconv covers the gap).
- num-bigint / num-integer: already deps (ap-obj/Cargo.toml:15-16); all
  int-side fixes reuse them.

## 4. Self-verification checklist

- [x] every P06/P07 primitive row has a verdict + gap analysis (bench ratio
      column = PENDING baseline (W2) placeholder, to be filled by master)
- [x] memchr-crate opportunity costed and flagged "user decision needed"
- [x] file:line cites for every "our impl" claim (read back, verified)
- [x] int(str) analysis distinguishes small-int vs bigint paths (§1.4:
      there is NO small fast path — num-bigint parse for every input)
- [x] audit-only: no file outside this doc written; nothing committed

## 5. Filed bugs discovered by this audit (for master to dispatch)

1. `int.to_bytes` on a bigint ≥ 2^63 silently produces zeros
   (builtins_modules.rs:16902-16903, `unwrap_or(0)`).
2. `int.from_bytes` with > 16 bytes silently truncates; `big_or_int`'s
   big arm is an admitted stub that casts to i64
   (builtins_modules.rs:19229-19246, 19661-19669).
3. (correctness-adjacent, not perf) bytes.find/index ignore the optional
   `[start[, end]]` args that CPython accepts — surface gap, may already
   be tracked elsewhere.
