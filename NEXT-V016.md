# V016 surrogate string support — leaf design (2026-11-19, worktree leaf-v016, branch tier1-v016 off 74ab286)

## Problem (root-caused by 03.07, V016-root-cause.md)
`"\ud800".encode("utf-8")` must raise UnicodeEncodeError (V016 corpus, the ONE
values-family red: 46/47). Two defects, both verified on trunk binaries:
1. ruff_python_parser 0.0.4 string.rs:179 maps every \u/\U escape in
   D800..DFFF to U+FFFD at PARSE time. Rust `char` cannot hold a lone
   surrogate, so the value is destroyed before any runtime guard runs.
2. `chr(0xd800)` raises ValueError (builtin_chr uses char::from_u32) where
   CPython returns a 1-char surrogate string. Verified: trunk chr-probe.py
   rc=1 vs CPython rc=0.

## Design: WTF-8 string storage (CPython-compatible at the seams)
Rust String cannot hold surrogates. Encode each lone surrogate code point
as its 3-byte UTF-8-style sequence (ED A0 80 .. ED BF BF) inside the
existing StrRepr::Unicode Box<str>. This is WTF-8. Valid UTF-8 never
contains those bytes, so every existing as_str() consumer keeps working
byte-identically on surrogate-free strings (the whole current corpus).
Surrogate-bearing strings are NEW territory: only the paths that must be
code-point-aware get updated.

### Layer 1 — ap-parse (parser shim, no vendor patch)
New module `crates/ap-parse/src/surrogates.rs`:
- `decode_literal(raw: &str, raw_string: bool) -> String`: re-decodes the
  literal CONTENT (ruff gives us content_range()) with CPython escape
  semantics, but emits WTF-8 for \u/\U escapes in D800..DFFF. All other
  escapes mirror ruff's parse_escaped_char exactly (incl. unknown escapes
  kept as backslash+char, octal, \N{name} via unicode_names2, line
  continuations dropped). Raw strings: content verbatim (no escapes).
- `patch_module(module: &mut ModModule, source: &str)`: ruff Visitor over
  the parsed tree; for every StringLiteral part (incl. implicit-concat
  parts and f-string FStringPart::Literal) and every
  InterpolatedStringLiteralElement (f/t-string middle chunks), replace
  `value` with decode_literal(content). Only touches nodes whose content
  contains a `\u`/`\U` escape (fast pre-scan); everything else untouched.
  Called from parse() after try_into_module, before validate_module.
- f-string middles: ruff already processes escapes in
  parse_interpolated_string_middle; the middle's range covers the raw
  chunk, so re-decode from source with the same decoder (brace doubling
  {{ }} handled: middles arrive with doubled braces collapsed by ruff —
  re-decode must collapse them too; verify against probes).
- \N{...}: ruff resolves names via unicode_names2; the shim keeps ruff's
  resolved char for \N (names never map to surrogates) — decode \N by
  looking up the SAME crate (add unicode_names2 = 1.2.2 dep, already in
  Cargo.lock via ruff).
- Errors: malformed \u/\U/\x/\N in source already fail ruff's parse before
  the shim runs (ruff raises UnicodeError); the shim only re-decodes
  ACCEPTED literals, so it cannot introduce new parse errors. \U00110000+
  rejected by ruff already.

### Layer 2 — ap-obj (PyString code-point accessors)
value.rs additions (no repr change, no new variant):
- `PyString::from_code_points(cps: impl Iterator<Item=u32>) -> Gc<PyString>`
  (WTF-8 encode; used by chr()).
- `PyString::code_points(&self) -> Vec<u32>` / `iter_code_points` — decode
  WTF-8 (fast path: if bytes are valid UTF-8, chars() as u32).
- `len()` currently counts chars() — WTF-8 surrogate bytes decode as 3
  invalid bytes; fix len() to count code points via the WTF-8 decoder
  (ASCII fast path unchanged).
- `py_hash()`: hash the code-point sequence (CPython hashes UCS2/UCS4
  units; our documented contract is equal-content-equal-hash, so hash
  the WTF-8 bytes consistently — but chr(0xd800) and "\ud800" must hash
  equal: both produce identical WTF-8 bytes, so hashing bytes is fine).
- PartialEq already compares as_str() bytes — identical WTF-8 => equal. OK.
- values_cmp (dict.rs:807): str cmp must be CODE-POINT order, not byte
  order (WTF-8 surrogate bytes ED.. sort above U+FFFF UTF-8 F0.. — wrong).
  Compare via code-point iterators when either side is non-UTF-8
  (contains_surrogates() check on bytes: ED A0..BF prefix scan).
- str_repr.rs: escape surrogates as \ud800 (CPython repr rule: Cs category
  escaped). Current code passes non-ASCII through; add the surrogate arm.
  ascii() builtin (builtins.rs:4666) already escapes all non-ASCII — but
  it iterates chars(); route through code_points so a surrogate becomes
  \ud800 not three \xNN. Astral split-to-surrogate-pair logic there must
  NOT split real astral chars differently — CPython ascii() emits
  \U0001f600 for astral (probed!), current code emits surrogate pairs —
  PRE-EXISTING divergence, fix to \U form while touching (probe-verified).

### Layer 3 — ap-run (runtime surfaces)
- builtin_chr: accept 0..0x110000 incl. D800..DFFF via from_code_points.
- builtin_ord: code-point decode (WTF-8) instead of chars().
- methods.rs encode_str: signature stays (s: &str) but iterate CODE POINTS:
  * utf-8 strict: surrogate run detection with CPython exact messages —
    single: "character '\ud800' in position N", consecutive run:
    "characters in position N-M" (probed: start/end attrs).
  * utf-8 errors handlers: surrogatepass (emit ED.. bytes), ignore,
    replace ('?'), backslashreplace (\ud800), xmlcharrefreplace (&#55296;),
    namereplace (\ud800 for surrogates — probed), surrogateescape raises
    for D800..DBFF (only DC80..DCFF pass through as bytes).
  * ascii/latin-1: same handler set; strict messages probed.
  * utf-16/32 (+le/be): surrogatepass emits raw units; strict raises
    "surrogates not allowed"; BOM forms probed.
  * unknown handler: LookupError "unknown error handler name 'bogus'"
    (probed) — only when an error actually occurs.
- decode_bytes_err utf-8: strict rejects ED A0.. sequences with CPython
  text ("can't decode byte 0xed in position N: invalid continuation
  byte" — probed); surrogatepass decodes them to surrogate code points;
  surrogateescape maps each bad byte to U+DC00+byte (probed);
  backslashreplace emits \xNN per byte (probed: 3 escapes for ED A0 80);
  replace: one U+FFFD per bad byte (probed 3 for ED A0 80).
- builtins_modules.rs _codecs.utf_8_encode/utf_8_decode markers: same
  semantics (they feed the vendored codecs.py registry path).
- encode_utf16/32 + le/be markers: surrogate-aware (strict raise /
  surrogatepass raw units).
- str methods that iterate chars() on surrogate strings (upper/lower/
  replace/find/in/startswith...) — Rust str ops on WTF-8 bytes still work
  for ASCII-pattern ops because surrogate bytes are self-synchronizing;
  upper/lower of a surrogate = identity (CPython probed). Case ops on
  non-ASCII near surrogates: leave byte-level (documented residual).
- f-string render (interp.rs render_fstring): String push_str of WTF-8
  parts works; repr conversion routes through str_repr (fixed above).
- bytes(str) builtin (builtins.rs:5120): currently copies as_str bytes
  for utf-8 — must raise on surrogates (strict) like encode; route
  through encode_str. The hardcoded "position 0-3" ascii message there
  is a pre-existing wrong-bytes stub — fix to real position scan.

### Out of scope (filed, not fixed)
- WasmGC backend string model (Plan 03 closed; wasm str is i32-array of
  code points — surrogates there are a separate 03.x follow-up; V016 wasm
  leg stays NOT-LOWERED-adjacent, native gate is this leaf's exit).
- mbc CJK engines already handle surrogates via their error handlers
  (probed gbk ignore/replace works once code points reach them — the
  mbc encode iterates chars(); route through code points in mbc_encode).
- unicode_escape/raw_unicode_escape codecs decode side (encode side
  exists; decode of \ud800 escapes — check fallback path, fix if cheap).

## Gates (exit contract)
1. V016 corpus case byte-exact vs python3.14 (ap-conform values slice
   47/47, was 46/47).
2. New corpus case V048-surrogate-strings.py (corpus only grows): the
   probe matrix in surr-probe*.py, byte-exact.
3. chr-probe.py byte-exact (chr/ord/repr/encode round trip).
4. ap-parse unit tests: shim decode matrix (escapes, raw, concat,
   f-string middles, \N) — authored in-crate.
5. No regression: cargo test -p ap-parse -p ap-obj -p ap-run (heavy lane
   serialized), ap-conform values+stdlib slices vs recorded baselines
   (values 46/47 -> 47/47; stdlib 47/48 held; s016 stays red, documented).
6. pytest tests/t05_values + t01_parse slices green.

## Sibling isolation
Live siblings: leaf-t121b (threading/timing + bisect), leaf-0405
(ap-cli/ap-aot-crt/ap-host hybrid), leaf-d13 (ap-cli main.rs/lib.rs).
This leaf owns: ap-parse/*, ap-obj/{value,str_repr,dict}.rs,
ap-run/{builtins,methods,builtins_modules,mbc}.rs codec arms,
tests/corpus/values/V048*, plans docs. Zero overlap.


---

## FINAL GATE RECORD (2026-11-20, worktree leaf-v016)

- values corpus slice: **48/48 PASS** (was 46/47; V016 GREEN + new V048
  surrogate-strings case GREEN, authored from the probe matrix first).
- Probe matrix: 9 surr-probe scripts (70+ cases: encode handlers x
  utf-8/ascii/latin-1/utf-16/32 + BOM forms, decode handlers, chr/ord/repr/
  len/index/cmp/hash, CJK gbk paths, error start/end attrs) ALL byte-exact
  vs python3.14 through the debug binary.
- Unit tests: ap-parse surrogates 8/8, ap-obj wtf8 2/2 + full lib green,
  ap-run surr_codec 10/10 + full lib green.
- Full corpus: 329 cases, 308 pass / 21 fail — ALL 21 in documented
  pre-existing families (classes metaclass remainder, codecs_cjk gen tools,
  f33 bytes-adjacent, gc018, s016 parser-lineno, T109 threading, t1_007);
  verified pre-existing against the trunk baseline run. Zero new reds.
- En-route real-bug fixes (all probed before fixing):
  1. mbc CJK engines crashed with UB-check ABORT on surrogate input
     (char_indices -> invalid char); now code-point iteration, exact
     CPython error texts (gbk ignore/replace byte-exact).
  2. utf-16 BOM decode: BOM selects endianness AND the error-message codec
     name; positions are ABSOLUTE including BOM bytes (decode_utf16_off).
  3. utf_16/32_ex_decode: BOM sign vs native endianness (FE FF -> +1,
     FF FE -> -1; was hardcoded BE).
  4. UnicodeEncodeError/UnicodeDecodeError instances now carry start/end
     attrs parsed from the message shape (was AttributeError/None).
  5. str subscript on WTF-8: code-point indexing (chars() UB hazard).
- GOTCHA learned: `cargo fmt -p ap-run` REFORMATS the whole crate (trunk
  has 74 blocks of pre-existing fmt drift) and TRUNCATES the generated
  mbc_tables.rs (224k lines). Never fmt ap-run wholesale on this tree;
  the fmt churn was reverted, edits re-applied surgically.
- Reverted the accidental edit to MAIN checkout's interp.rs (shared trunk
  tree restored clean).

## Commit plan (this leaf)
1. ap-parse shim (surrogates.rs + lib.rs hook + Cargo.toml).
2. ap-obj wtf8 module + PyString code-point surfaces (len/hash/cmp/repr).
3. ap-run surr_codec engine + builtin codec routes + chr/ord/subscript +
   mbc cp loops + start/end attrs.
4. corpus V048 + NEXT-V016.md.
