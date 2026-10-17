# F-string differential bug hunt (parts-not-elements + debug forms)

Isolated investigation leaf, spawned from the pon-study flag
(docs/pon-study-lowering.md "f-strings / t-strings"). Reference:
CPython 3.14.7 (`python3.14`, 2026-10-16; provisional vs the C-01 pin
3.14.0, per the T-05/T-06/T-11 provenance-gap precedent). Battery:
`tests/corpus/fstrings/cases/*.py` (35 cases), expected triples in
`tests/corpus/fstrings/expected/` (stdlib layout: `.out`/`.err`/`.rc`),
gate tests `tests/fstrings_corpus/test_fstrings_corpus.py`.

Tooling under test: `ap-ir-dump <file>` (T-02 tree form), `--golden`
(dump_module), `--verify` (T-04 static pass). **No panics on any case in
any mode** (35 × 3 runs scanned for `panicked at`).

## Verdict summary

| # | case | form exercised | verdict |
|---|------|----------------|---------|
| 1 | f01 | same-quote nesting `f"... {"lit"} ..."` | OK (semantic equivalence) |
| 2 | f02 | dict subscript with both quote styles | OK |
| 3 | f03 | backslash inside replacement field | OK |
| 4 | f04 | `{x:.2f}` | OK |
| 5 | f05 | `{x:>10}` | OK |
| 6 | f06 | `{x:^10}` | OK |
| 7 | f07 | `{x:*>10}` | OK |
| 8 | f08 | `{x:+d}` sign | OK |
| 9 | f09 | `{x:#x}` `{x:#X}` `{x:#o}` | OK |
| 10 | f10 | `{x:#b}` | OK |
| 11 | f11 | `{x!r}` | OK |
| 12 | f12 | `{c!s}`/`{c!r}` class conversion | OK |
| 13 | f13 | `{x!a}` ascii | OK |
| 14 | f14 | nested f-string 2 deep | OK |
| 15 | f15 | nested spec `{v:{8}.{3}}` | OK |
| 16 | f16 | runtime width `{x:{w}}` | OK |
| 17 | f17 | `{x=}` debug plain | **BUG-1** (information loss) |
| 18 | f18 | `{x = }` debug spaced | **BUG-1** (information loss) |
| 19 | f19 | `{x=:>10}` debug+spec | OK (spec kept; see BUG-1) |
| 20 | f20 | `{x=!r}` debug+conv | OK (conv kept; see BUG-1) |
| 21 | f21 | `{d['k'] = }` debug expr | **BUG-1** (information loss) |
| 22 | f22 | triple-quoted multiline f-string | OK |
| 23 | f23 | multiline replacement field | OK |
| 24 | f24 | dict literal in field | OK |
| 25 | f25 | lambda call in field | OK |
| 26 | f26 | walrus in field + use after | OK here; see FINDING-3 (verify) |
| 27 | f27 | conditional expr in field | OK |
| 28 | f28 | method call / slice / join in field | OK |
| 29 | f29 | generator exprs in field | OK |
| 30 | f30 | escaped braces `{{ }}` | OK |
| 31 | f31 | implicit concat: plain `"…"` BETWEEN f-parts (pon gotcha) | **OK — parts-not-elements NOT a bug in us**; see FINDING-3 (verify) |
| 32 | f32 | plain literal between f-parts, tab in Lit tail | OK |
| 33 | f33 | `f"…" b"…"` adjacent bytes | OK (parse refusal matches CPython SyntaxError; byte-diff of message text = FINDING-4) |
| 34 | f34 | nested spec mixed literal+expr `{x:>{w}.2f}` | OK |
| 35 | f35 | empty-string field, self-nested empty f-string | OK |

**Bug count: 1 real lowering/IR bug (BUG-1, medium), 1 dump-fidelity gap
(DUMP-1, cosmetic), 1 out-of-scope verify finding (FINDING-3, not
f-string), 1 diagnostic-text contract note (FINDING-4).**

## BUG-1 (medium): `{x=}` debug text and default-`r` conversion are dropped in lowering

The pon-flagged suspicion, confirmed — but it is the **debug-text** leg,
not parts-vs-elements, that is the live bug.

CPython semantics (S-01-16/C-18; PEP 701 debug forms):

- `f"{x=}"` renders the literal source text `x=` followed by
  `repr(x)` — the debug literal text is REAL emitted output.
- `f"{x = }"` preserves the inner whitespace: emits `x = ` + `repr(x)`.
- `f"{x=!r}"` keeps the explicit conversion but still emits `x=`.
- `f"{d['k'] = }"` emits `d['k'] = ` + `repr(...)`.
- With no format spec and no explicit conversion, the debug form implies
  conversion `r`; with a spec (`{x=:>10}`) there is NO implied `r`.

What ruff gives us: `InterpolatedElement::debug_text: Option<DebugText>`
(`DebugText { text, expression_start, expression_end }` — full source
text between `{` and the conversion/spec/`}`, plus expression offsets).
Ruff sets `conversion: ConversionFlag::None` for bare `{x=}` — the
implied-`r` is NOT baked into the flag.

What our lowering does (`crates/ap-parse/src/lowering.rs`,
`lower_interp_element`): reads only `interp.conversion.to_char()` and
`interp.format_spec`; `debug_text` is never read. Result:

- `f"{x=}"` lowers to `Interp { expr: x, conv: None, spec: None }` —
  the literal `x=` is gone and the implied `r` is gone.
- `--golden` renders `f"{x}"` where CPython would print `x=42`.

Minimal repro (corpus case f17):

```python
x = 42
print(f"{x=}")     # CPython: `x=42`;  our IR is FString[Interp{x}] → would print `42`
print(f"{x = }")   # CPython: `x = 42`
d = {"k": [1, 2]}
print(f"{d['k'] = }")  # CPython: `d['k'] = [1, 2]`
```

Dump evidence (`--golden`): all five debug cases render as if written
without the `=` (`f"{x}"`, `f"{x!r}"`, `f"{x:>10}"`). Cases f19/f20
happen to keep enough (spec kept, explicit conv kept) but STILL lose the
debug literal text, so their runtime output would be wrong too.

Why it is IR-level, not dump-level: `FStrPart::Interp`
(crates/ap-ir/src/expr.rs) has no field for the debug text. The
information is destroyed at lowering; no formatter can recover it.

### Recommended fix sketch (NOT implemented, per leaf scope)

1. **ap-ir** (`crates/ap-ir/src/expr.rs`): extend
   `FStrPart::Interp` with `debug_text: Option<String>` (the full
   `text` slice; keep expression offsets out of the IR — they are
   span/source-map concerns, and ap-parse already keeps source maps).
2. **ap-parse** (`lowering.rs`, `lower_interp_element`): read
   `interp.debug_text.as_ref().map(|d| d.text().to_string())` into the
   new field.
3. **Conversion default**: at the same site, when `debug_text.is_some()`
   AND `conversion == None` AND `format_spec.is_none()`, set
   `conv = Some(Conversion::R)` — exactly pon's rule
   (`lower/strings.rs` debug-text handling), matching CPython. Note the
   precedence: spec present ⇒ NO implied `r` (f19 keeps `:>10` with no
   `r`); explicit conv present ⇒ keep it (f20).
   Alternatives considered: (a) bake `r` in at parse level — rejected,
   ruff already gives us the raw flag and re-deriving is trivial; (b)
   model debug forms as separate Lit+Interp parts — rejected, it would
   duplicate the expression source into a Lit and break round-tripping
   of `x = ` spacing through the structured IR.
4. **Renderers**: `dump_fstr_part` (crates/ap-ir/src/dump.rs) writes
   `debug_text` then `!flag` then `:spec`; the future runtime
   BuiltString lowering emits `Lit(debug_text)` before the Interp part
   (or stores the text on the interp instruction — Plan 02 choice).
5. **t-strings**: same field will carry t-string debug text when
   t-strings enter the subset; pon's `TEMPLATE_LITERAL_CONVERSION`
   sentinel is the reference for the "no conversion" wire encoding.

## DUMP-1 (cosmetic): multi-line Lit parts render raw newlines inside the one-line dump form

`dump_fstr_part` emits `Lit` text verbatim (crates/ap-ir/src/dump.rs);
a triple-quoted f-string's embedded `\n` (f22) or a `\n` literal chunk
(e3 probe) splits the dump across lines. The IR is faithful; the dump
is just ambiguous to eyeball. Suggested fix when convenient: escape
`\n`/`\t`/`\\` in `FStrPart::Lit` rendering (mirror the existing
`{{`→brace escaping already there). No repro file needed beyond f22;
recorded for the runtime era when dumps become diff surfaces.

## FINDING-3 (pre-existing, NOT f-string): walrus binds never become definite in the verify must-analysis

f26/f31 fail `--verify` with false-positive
`NameError: name 'w' is not defined` where CPython runs clean. Probing
shows this is NOT f-string-specific:

- `print(f"{(v := 42)}") ; print(v)` → flagged (f26 shape)
- `print(abs((v := 42))) ; print(v)` → flagged
- `z = (v := 42) ; print(v)` → flagged
- `if (n := len(a)) > 0: print(n)` → flagged
- walrus in a comprehension iterable/elt → NOT flagged (bind path via
  `bind_comp_target_definite`)

Root cause: `collect_expr_loads` (`crates/ap-ir/src/verify.rs`,
`Expr::Named` arm ~line 1607) calls `scope.bind(n)` — the INDEFINITE
bind — for walrus targets, while every other binding form goes through
`bind_definite`. `resolve_scope` requires `has_binding_before`, which
reads only `definite_binds`; a walrus bind is therefore never
protecting, and any later read of the name is a false positive
(`tests/fixtures/parse/valid_walrus.py` also trips this today if run
through `--verify`). Fix sketch: in `Expr::Named`'s arm, call
`scope.bind_definite(n)` after collecting the value loads (value loads
first — CPython evaluates `t := v` left-to-right). This is a verify.rs
change outside this leaf's no-touch set only in the sense that it is
not lowering.rs/ap-obj; it is NOT part of the f-string hunt and is
filed here because the battery surfaced it. f26/f31 keep rc=3
expectations pinned so the corpus records today's behavior until the
verify fix lands — flip them to rc=0 when it does.

## FINDING-4 (note): f33 parse-refusal text vs CPython SyntaxError shape

`f"{x}" b"tail"` is a SyntaxError in both. Ours: parse/lowering refusal
`error at line 3, column 7: Bytes literal cannot be mixed with
non-bytes literals` (rc 1), verify mode rc 3 with the same message
prefixed `SyntaxError:`. CPython emits the standard traceback form.
Since the corpus pins OUR diagnostic contract (T-01 shape) rather than
CPython's traceback bytes, the expected triple records rc 1 + our
message. No action; noted so the runtime era doesn't mistake this for a
regression.

## Parts-not-elements (the pon gotcha): NOT a bug in us

`lower_fstring_value` iterates `FStringValue::iter()` — the PARTS list
(`FStringPart::Literal(StringLiteral) | FString(FString)`) — not any
`elements()` helper. The exact pon repro shape
`'"#define X {" f"{cname};" "%s"'` (case f31) and the
`f"{x}" "mid" f"{y}"` plain-between shape (case f32) both keep every
literal chunk, in order, as `FStrPart::Lit`. Interleaved literals, tab
and newline Lit tails, and quote-containing literals all round-trip.
The pon warning applies to code that walks `elements()` per FString;
our single walk over parts is already the correct shape.

## Corpus slice

- `tests/corpus/fstrings/cases/*.py` — 35 cases (f01..f35), each with a
  header comment citing the clause ids it pins.
- `tests/corpus/fstrings/expected/*.{out,err,rc}` — generated from live
  python3.14 (3.14.7) runs; re-verified byte-stable under `-I`.
- `tests/fstrings_corpus/test_fstrings_corpus.py` — T-11-style gate:
  triple exists, reference re-run byte-stable, determinism lint, and
  two coverage assertions (debug forms present in all four spellings;
  the pon mixed-concat shape present). 107/107 green.
- `tests/corpus/fstrings/gen_cases.py` — the one-time generator (kept
  for provenance; expected triples are checked in, not generated at
  test time).
- Nothing wired into ap-conform globs or CI manifests (per leaf
  instructions); the layout convention is what ap-conform already
  understands.
- NOTE: the background `cargo test -p ap-ir --test verify` + full
  pytest run launched during this leaf completed 121 passed / 1 failed
  (the transient f30 generator/case divergence, since fixed) / 3
  skipped / 82 xfailed / 43 xpassed — re-run after the f30 fix shows
  the fstrings slice 107/107 green. No pre-existing suite was touched.
