# T-05 — values (gates implementation leaf 02.01 ap-obj core values)

Authorship: written from the spec pack ONLY (`docs/spec/01-values-and-objects.md`,
`docs/spec/06-observable-behavior.md`, `docs/compat-contract.md`,
`plans/antipythany/master.md`, leaf 02.01's Expose/Gate in
`plans/antipythany/02-native-backend/orchestrator.md`), per
`plans/antipythany/tests/README.md`. The author has not read any implementation
source under `crates/`.

## Purpose

Gate leaf 02.01 (`ap_obj::Value` core values + ops trait surface) on the
**values corpus slice**: every MUST clause of spec 01 (S-01-1 .. S-01-45) that
is observable through `print`/repr/error-text on builtin values gets at least
one pinned case; contract clauses C-12..C-18 and C-26 are cited where they bind
the same bytes. Leaf gate (from 02.01): "ap-conform green on the values corpus
slice".

## Testability contract under test

- `ap-conform run --corpus <glob> --impl <cmd>` diffs **stdout bytes, stderr
  bytes, exit code** per case (S-06-1/S-06-2; contract C-24/C-27). The pinned
  expectations in `tests/corpus/values/expected/` were produced by running each
  case as `python3.14 <abs-path>` on this machine's `/opt/homebrew/bin/python3.14`
  (3.14.7 — see CONTRACT-GAPS G-0 on the C-01 patch pin).
- The slice is CORPUS-GENUINE: cases are plain `.py` scripts under
  `tests/corpus/values/`; they grow monotonically and are never weakened (C-33).
- Until 02.01 lands, the antipythany-side tests in `tests/t05_values/` are
  `xfail(strict=False)`; the fixture self-checks (reference re-run byte-compare,
  triple presence, header-citation lint, salted-hash lint, corpus-prohibited-API
  lint, harness self-comparison through `ap-conform --impl python3.14`) run and
  MUST stay green — they test the fixtures and the harness, not antipythany.
- Invocation overrides for CI: `AP_CONFORM=<path>` (default
  `target/debug/ap-conform`), `APX_IMPL="<cmd…>"` (the impl command line ap-
  conform-style, e.g. `target/debug/antipythany run`).

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t05_values/ -q
```

From the repo root. Current status: **192 passed, 7 xfailed, 0 errors** (192 =
fixture self-checks incl. the ap-conform self-comparison; 7 = antipythany-side
gate params).

## Corpus case table (tests/corpus/values/)

| Case | Cites | Pins (one line) |
| --- | --- | --- |
| V001-bigint-exactness.py | S-01-1, C-12, C-13 | underscores/radix literals; +,-,*,shift,and/or/xor,~ on >200-bit values; arithmetic right shift of negatives |
| V002-power-rules.py | S-01-2, C-14 | exact 2**200, 0**0, negative exponent→float, `-7**2`/right-assoc, `0**-1` ZeroDivisionError text, `1<<(2**62)` MemoryError-not-OverflowError |
| V003-floordiv-mod-signs.py | S-01-3, S-01-4, C-12 | full // and % sign table for int + float analogue + exhaustive a==(a//b)*b+(a%b) sweep |
| V004-divmod-mixed.py | S-01-5 | divmod tuple == (//,%) incl. mixed int/float promoted result types |
| V005-true-division.py | S-01-6, S-01-9 | `/` always float (`6/3`→2.0); bool arithmetic results are int; `not` is bool |
| V006-float-ieee-overflow.py | S-01-8, C-16 | 0.1+0.2 last-bit, silent `1e308*10`→inf, `1e999`, inf-inf→nan, float 0-division/modulo error texts AS MEASURED (see G-1) |
| V007-int-float-promotion.py | S-01-7, C-12 | mixed-type promotion is float; 2**53+1 exactness vs float-side rounding; += retypes |
| V008-bool-is-int.py | S-01-9, S-01-10, S-01-32 | isinstance(True,int), True==1, type names, repr True/False/None, `t is n` False via variables (see G-4) |
| V009-float-to-int-errors.py | S-01-11, C-14 | int() truncates toward zero (±3.7, -0.0); NaN/infinity exact texts; float(10**400) OverflowError; 2**10000 does NOT raise |
| V010-round-bankers.py | S-01-12, S-06-18 | ties-to-even incl. below-tie 2.675→2.67, 0.125→0.12; ndigits retypes int/float |
| V011-int-string-parsing.py | S-01-13 | int(str) whitespace/sign/underscores, bases 2/16/32/36, exact ValueError texts for bad int/float strings |
| V012-str-codepoints-errors.py | S-01-14 | len counts code points (NFC 4 vs NFD 5, not ==); item-assign IndexError/TypeError texts via variable keys (see G-4) |
| V013-slice-algorithm.py | S-01-15 | negative bounds, silent clamp both ends, backward negative step, `step 0` ValueError, same-type-out on str/list/tuple/bytes |
| V014-fstring-formats.py | S-01-16 | !s/!r/!a, pad/zero-pad/comma specs, `expr=` debug form prints source text, nested same-type quotes, nested f-string, `{v=!r}` combo |
| V015-str-bytes-separation.py | S-01-17 | both concat-direction TypeError texts (distinct!), non-int multiplier text, `in` works within type |
| V016-encode-decode-errors.py | S-01-18 | UTF-8 round-trip, default codecs, bytes→int indexing, byte-count vs code-point len, the three codec-error exact texts |
| V017-list-aliasing-mutation.py | S-01-19 | `ys=xs` aliasing, `+` builds new vs `+=` mutates, recursive ==, del, tuple `del t[0]` TypeError text |
| V019-tuple-shapes.py | S-01-20 | `(1,)` vs `(1)` vs `()`, star-unpack order, item-assign TypeError |
| V040-dict-insertion-order.py | S-01-21 | iteration/keys/values/items/list(d)/repr order; re-assign keeps original position |
| V041-dict-equality-order-free.py | S-01-22 | == ignores order, requires length and == values |
| V042-dict-collision-keys.py | S-01-23, S-01-35 | 1/1.0/True one key-class, value overwritten to last, FIRST insertion position kept, len 1 |
| V043-dict-key-errors.py | S-01-24 | display-key and subscript-key unhashable texts (type named twice), KeyError str == repr(key) |
| V044-set-ops-unordered.py | S-01-25, C-26 | set ==/membership/`\| & - ^ <= <` order-independent; MULTI-element order MASKED via sorted(); `{}` is dict; single-element repr stable |
| V045-frozenset-hashable.py | S-01-26 | frozenset ==/hash element-order-independent in-process, usable as key/element, empty+single reprs |
| V046-set-element-error.py | S-01-26 | set-display unhashable uses set-element phrasing (distinct from dict-key); set.add/frozenset() plain unhashable text |
| V047-truthiness-table.py | S-01-27 | exhaustive falsy table (False/None/all numeric zeros/all empties incl. b"" set() frozenset() 0j -0.0) and truthy "0" [0] (None,) nan |
| V048-bool-protocol.py | S-01-28 | bool(x) prefers `__bool__` over `__len__`; `__len__`==0 falsy; plain object truthy |
| V049-and-or-operands.py | S-01-29 | and/or return the OPERAND not bool; short-circuit proven with side-effect probe |
| V050-not-bool.py | S-01-29 | `not` always real bool, even over custom-`__len__` objects |
| V051-eq-vs-identity.py | S-01-30/31/32 | `is` used only on None/sentinel/name-to-itself (caching UNSPECIFIED — masked away); None != False/0; None is None |
| V052-cross-type-numeric-eq.py | S-01-33 | 1==1.0==True, 0==-0.0, structural recursion through list/tuple/dict; unrelated types == is False not error |
| V053-nan-equality-containment.py | S-01-33 | nan!=nan; containment finds same nan via identity-first in list/set/dict; hash(nan) NEVER printed (UNSPECIFIED) |
| V054-chained-comparisons.py | S-01-34 | `1 < "x"` exact TypeError text; chained comparison evaluates operands at most once + short-circuits (side-effect log `[1,2]` proves c(0) skipped) |
| V055-hash-eq-invariant.py | S-01-35 | == implies hash== hash across int/float/bool incl. hash(0)==hash(-0.0)==0 |
| V056-hash-int-contract.py | S-01-36 | hash(42)==42, hash(-1)==-2, 2**61-1 modulus for ±10**100, hash(modulus)==0, sys.hash_info pinned |
| V057-hash-float-contract.py | S-01-37 | integral float hashes as int, 0.0/-0.0→0, hash(inf)==314159, hash(-inf)==-314159; nan excluded |
| V058-salted-hash-masked.py | S-01-38, S-06-16 | str/bytes hashes are salted UNSPECIFIED — VALUE NEVER printed (masking decision); only in-process equal-hash + keyed-container consistency asserted |
| V059-hashable-types.py | S-01-39 | v1 hashable roster (int,bool,float,str,bytes,tuple,frozenset,range,class) all hash to int; list/dict/set unhashable exact texts; nested-tuple list propagates |
| V060-repr-int-decimal.py | S-01-40, S-01-1 | repr(int) digits-only, repr(-0)==0, str==repr, `,`/`+d` formats don't alter value bytes |
| V061-float-repr-shortest.py | S-01-41, C-15, C-17 | full shortest-round-trip layout grid: 1e+16/1e15 boundary, two-digit exponents, 5e-324, inf/-inf/nan, -0.0, 1/3 and 10/3 digits, round-trip property |
| V062-str-equals-repr-and-float.py | S-01-41 | str(float)==repr(float); print emits repr layout |
| V063-str-repr-quotes-escapes.py | S-01-42 | quote-selection rule all three branches, `\t\n\r\\` + lowercase `\xNN` controls, non-ASCII passthrough (é 中 😀), eval(repr)==s, print≠repr |
| V064-bytes-repr.py | S-01-43, S-01-18 | b'…' repr with same quote rule, `\xNN` for non-printable/non-ASCII bytes, raw printable ASCII |
| V065-container-repr-cycle.py | S-01-44, S-01-23 | recursion + exact punctuation (`{1: 'x'}` collision fold, 1-tuple comma), str==repr for containers, all empty reprs, self-list `[1, [...]]`, self-dict `{...}` |
| V066-default-object-repr-masked.py | S-01-45 | default repr `<module.Class` prefix printed, ADDRESS MASKED (split at ' object at 0x', never emit the hex — UNSPECIFIED); str fallback to __repr__; __str__/__repr__ divergence; containers recurse with repr |
| V067-int-str-digit-limit.py | C-14, S-01-1 | int↔str 4300-digit limit: both exact measured texts (str-form has no digit count — see G-2), set_int_max_str_digits raises it, 2**10000 prints 3011 digits unharmed |
| V068-float-format-specs.py | C-18, S-01-16 | format spec matrix over the C-15 boundary value set × ['',e,.6f,>12.4g,n,010.2%,.0f], printed deterministically row-by-row |

Coverage: every MUST clause S-01-1..S-01-45 is cited by at least one case
(S-01-31 is covered by *avoidance* — V008/V051 keep `is` off cross-object
literals per its own prohibition). C-12..C-18 covered by V001-V011, V060-V063,
V067, V068; C-26 by V040/V044; C-14 by V002/V009/V067.

## Deterministic-masking decisions (UNSPECIFIED → masked, never deleted)

1. **Set/frozenset element order** (S-01-25, C-26): multi-element set observables
   go through `sorted(s)` or membership/equality; only empty and single-element
   set reprs are byte-pinned. V044, V045.
2. **Salted str/bytes hashes** (S-01-38, S-06-16): no case prints `hash("…")` or
   `hash(b"…")` values; V058 pins only the in-process equal-string→equal-hash
   contract, and a lint test (`test_no_salted_hash_printed`) enforces it
   suite-wide.
3. **hash(nan)** (S-01-37 UNSPECIFIED): never computed into output; V053 pins
   nan equality/identity-containment only.
4. **Object addresses** (S-01-45, S-06-17 #4): V066 partitions the default repr
   at ` object at 0x` and prints only the stable class-prefix; hex digits are
   dropped. (S-06-17 #4 actually *prohibits* plain-object repr as an anchor —
   the masked-prefix form is the compliant weakening of that pin.)
5. **Caching/interning `is`** (S-01-31): no cross-object `is` assertions; `is`
   appears only on None, a name vs itself, and (V008) two *variable-bound*
   values True/1 whose non-identity S-01-32 pins as semantics, not caching.
6. **3.14 SyntaxWarning pollution** (see G-4): `"abc"[1.5]` and `True is 1`
   *literal* forms print warnings to stderr on the reference before the run
   starts; to keep this slice's stderr pins clean, those forms route through
   variables — the runtime TypeError/identity bytes are still pinned.
7. **NaN bit patterns, float repr of runtime-computed values**: all pinned
   floats are compile-time literals or deterministic ops (0.1+0.2, 1/3);
   `-0.0` printed only via repr (V010 avoids printing repr(round(-0.0)) sign
   dependence by pinning the measured bytes).

## CONTRACT-GAPS

- **G-0: patch pin vs live reference (C-01).** C-01 pins conformance to CPython
  **3.14.0**; the only binary on this machine reports 3.14.7 (`-VV`). Per C-01's
  own text, expectations recorded from a non-pinned patch are *provisional*.
  These triples were produced by real 3.14.7 runs because no 3.14.0 build
  exists here. Resolution owed: re-generate (or diff-verify) all expected bytes
  against a 3.14.0 build before the 02.01 gate is *certified*; the mismatch-
  override path must label such runs non-normative. No corpus semantics change
  is expected — this is a provenance gap.
- **G-1: C-16 float-div-zero message text is stale.** Compat contract C-16
  claims `1.0/0.0` raises `ZeroDivisionError: float division` and `1.0%0.0`
  "float modulo". Live reference (3.14.7) raises **`division by zero`** for
  `1.0/0.0`, `0.0/0.0`, `1.0%0.0`, `0.0%0.0`, `0.0//0.0`, and `divmod(1.0, 0)` —
  identical to the int text, exactly as spec-01 S-01-8 states ("raises for BOTH
  operand types with identical text"). Spec pack and reference agree; the
  contract's example quotes conflict. Per C-02 (reference observes win) and the
  tests/README protocol ("never silently pick semantics"), filed here for a
  contract PR. Cases V002/V006 pin the measured texts.
- **G-2: C-14 digit-limit message is quoted incomplete.** C-14 writes
  `ValueError: Exceeds the limit (4300 digits) for integer string conversion`
  with no trailing clause. Measured: the **str(int)** form appends `; use
  sys.set_int_max_str_digits() to increase the limit` and has NO "value has N
  digits" segment, while the **int(str)** form includes both (`: value has 5000
  digits; use …`). Two distinct texts; V067 pins both exactly. Contract PR
  should quote both forms.
- **G-3: `0 ** -1` text.** Spec S-01-2 quotes `zero to a negative power`;
  verified live (matches). (An earlier authoring guess "…negative integer
  power" was wrong; the pinned bytes follow the spec quote. No gap — recorded
  so reviewers know it was checked.)
- **G-4: SyntaxWarning channel for S-01-14/S-01-31 literal forms is not gated
  by this slice.** The spec says CPython 3.14 warns for `is`-with-literal and
  for float-indices-look-like-missing-comma, and S-01-31 says antipythany
  SHOULD warn identically. Pinning *that stderr text* byte-exact belongs to the
  parse-diagnostics slice (T-01), not values: inside this slice those forms
  were routed through variables (masking decision #6) so that values cases keep
  empty stderr. Gap: no corpus case in ANY existing slice yet pins the two
  warning texts; recommend T-01 author claim them.
- **G-5: S-01-45 address masking vs S-06-17 #4 prohibition.** S-06-17 bans a
  default-`__repr__` object print as an anchor outright; S-01-45 says "tests
  compare only the stable prefix". V066 follows S-01-45's masked-prefix form.
  The two clauses are reconcilable but the spec pack should state that
  prefix-only masked printing is the sanctioned way to pin S-01-45.
- **G-6: `f"{v=!r}"` combo form.** S-01-16 pins `f"{v=}"` and `!r` separately;
  the *combination* (debug text + conversion) is not spelled out. Measured
  bytes are pinned in V014 (`v=5` — the spec example's shape applies the
  conversion to the value, not the debug label). Flagged as an interpretation
  point, not a contradiction.
- **G-7: `eval(repr(s))` round-trip in V063.** `eval` is a builtin whose IR
  semantics belong to spec 02/05; its use here is as an oracle *inside the
  reference run only* (the corpus pins the printed `True`/`False` summary
  bytes, not eval itself). If exec/eval is out of 02.01's scope the case still
  conforms because the pinned channel is just the boolean line. Noted for the
  02.01 gate review.

## xfail policy

Same as T-02: all antipythany-side tests are `xfail(strict=False, reason=
"awaits 02.01 ap-obj core values (antipythany impl side)")` — 7 of them (full-
slice ap-conform run + 6 spot cases diffed channel-by-channel against pinned
triples). Until 02.01 lands the suite shows xfail, zero hard failures = green
for this test leaf. After landing, xpass is expected; any FAIL is a 02.01
implementation bug — fix the impl, never the corpus (C-33 / AGENTS.md).
