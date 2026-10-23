# T1 regex risk map — scrapy closure vs unfixed re-engine bugs (RF-2/3/5/7)

Read-only investigation 2026-10-22. Trees swept:
- `~/.cache/antipythany/site-packages` (scrapy 2.19.0, w3lib, parsel, itemadapter,
  itemloaders, protego, jmespath, cssselect, queuelib, six, pydispatch, zope)
- `/private/tmp/gatecheck/cache/site-packages` (same + the **twisted 26.4.0**,
  attr/attrs, automat, constantly, incremental, hyperlink, idna, packaging,
  pyasn1(_modules), service_identity, setuptools tree the T1 gate actually runs —
  the `~/.cache` staging lacks twisted; both trees swept, results identical for
  the scrapy-tree packages)
- `/Users/caimlas/git/antipythany/vendored_stdlib`

Method: full-text multiline call-shape sweep (`/tmp/rf2_sweep.py`,
`/tmp/regex_sweep.py`, stdlib-only) + byte probes: CPython
`/opt/homebrew/bin/python3.14` (3.14.7) vs
`/Users/caimlas/git/antipythany/target/debug/ap-run-eval` on
`/tmp/probeA_rf2.py`, `/tmp/probeB_rf3.py`, `/tmp/probeC_rf5_rf7.py`,
`/tmp/probeD_rf3_real.py`, `/tmp/probeE_htmlparser.py`. Raw outputs:
`/tmp/probe_out.txt`.

NOTE the impl resolves `import re` to the **native** module
(`re.__file__` is `None` under ap-run-eval); the vendored
`vendored_stdlib/re/__init__.py` pure-Python wrapper is shadowed, so its
`count=_zero_sentinel` kwargs plumbing does not run — the engine itself decides.

## Summary verdict

| Finding | On the file:// crawl path? | Proven divergence? | T1 severity |
|---|---|---|---|
| RF-2 count= kwarg | **NO** — zero `count=` kwargs anywhere in closure+stdlib; every closure `.sub()` is positional | YES but only via `count=` kwarg (probe A4/A5/A7) | latent (crash-free, wrong-bytes only if new code uses kwarg) |
| RF-3 lookbehind @0 | **NO** — only lookbehind in closure is itemadapter's literal *string list* (no regex); vendored `html.parser` attrfind is never matched at pos 0 and the full-parser round-trip is byte-identical (probe E); `logging.handlers` extMatch diverges at pos 0 but only fires with `LOG_FILE`+TimedRotating (verify) | YES (probes B1–B4, D2–D3) | low; one verify-flagged site |
| RF-5 lastindex | **NO** — `w3lib._util:87` IS on the crawl path (encoding/html sniffing) but its alternation-outside-repeat shape probes byte-identical; `ntpath.expandvars` shape also identical | NO divergence on the real patterns (C3–C6) | none observed |
| RF-7 repeat error shapes | **NO** — no consecutive-quantifier pattern string exists in either tree; all engine-compile happens at import time on valid patterns | YES on synthetic bad patterns (C7) but never triggered by closure | none |

**Bottom line: none of RF-2/3/5/7 currently blocks or corrupts the T1 scrapy
crawl.** The known T1 blocker remains lxml (02.08-c shim). Residual risks
are listed per-row below.

---

## RF-2 — `sub`/`subn` ignore `count=` keyword

### Sites found
- Module-level and pattern-method `count=` kwargs: **none** in
  `~/.cache/antipythany/site-packages/**/*.py`,
  `/private/tmp/gatecheck/cache/site-packages/**/*.py`, or
  `vendored_stdlib/**/*.py`. Sweep = paren-matched multiline call scan for
  `.sub(`/`.subn(` args containing `(?<![A-Za-z0-9_])count\s*=`
  (`/tmp/rf2_sweep.py`; exit 0, `[]` hits). Cross-checked with a raw
  `grep -rn 'count=' … | grep sub` — only hits were twisted *test* files
  using unrelated `dict(count=…)`/`trace.Trace(count=1)`
  (twisted/logger/test/test_filter.py:67, twisted/scripts/trial.py:317) and
  `str.split(maxsplit=…)`-family tokens. Test-only, not on crawl path.
- Positional-count users on the path (these exercise the WORKING leg):
  - `vendored_stdlib/http/cookiejar.py:634` `cut_port_re.sub("", host, 1)`
  - `vendored_stdlib/urllib/robotparser.py:236-237,338,344,346` (via
    protego? no — robotparser is stdlib, scrapy uses **protego**; both fine)
  - `vendored_stdlib/re/__init__.py:225,255` — shadowed by native `re`, N/A
  - `vendored_stdlib/ntpath.py` `_varsub = …re.compile(...).sub` then
    positional calls (kwarg-free)
  - `w3lib/html.py:222,251,268,336,382`, `email/generator.py:263…` — all
    bare `.sub(repl, text)` = unlimited replace, count not involved.

### Probe evidence (probeA_rf2)
| probe | CPython | impl | verdict |
|---|---|---|---|
| A1 `re.sub('a','-','aaa',count=1)` | `'-aa'` | `'-aa'` | OK (module-level splice fix in builtins_modules.rs:1680 holds) |
| A2 `re.subn(…count=1)` | `('-aa',1)` | `('-aa',1)` | OK |
| A3 positional `re.sub('a','-','aaa',1)` | `'-aa'` | `'-aa'` | OK |
| **A4 `p.sub('-', 'aaa', count=1)` (p=compiled)** | `'-aa'` | `'---'` | **DIVERGES** — pattern-method kwarg still dropped (sre path reads `rest.get(2)` positionally; rest has no 3rd elem → count=0 → all) |
| **A5 `p.subn('-', 'aaa', count=1)`** | `('-aa',1)` | `('---',3)` | **DIVERGES** |
| A6 `p.sub('-', 'aaa', 1)` positional | `'-aa'` | `'-aa'` | OK |
| **A7 bound `re.compile(r'(\d)').sub(f, s, count=1)`** | `'a1!b2'` | `'a1!b2!'` | **DIVERGES** (same pattern-method hole) |
| A8 `re.split(…,maxsplit=1)` | `['a','b,c']` | `['a','b,c']` | OK |

### T1 assessment
Crawl path: **not executed** (no kwarg callers in the closure). If any future
closure/two-line change (e.g. spider code in the gate app, or the 02.08-c
lxml-shim internals if they call `Pattern.sub(..., count=)` from Python) uses
it → **WRONG BYTES silently** (over-replacement in item text). Severity:
latent silent-wrong-bytes; zero today.

## RF-3 — fixed-width lookbehind at subject position 0

### Sites found (pattern strings containing `(?<=`/`(?<!`)
| site | shape | on crawl path? |
|---|---|---|
| `itemadapter/_json_schema.py:108-109` | `"(?<="` is a **plain string in INVALID_PATTERN_SUBSTRINGS** — a *rejection* list checked with `in`, never compiled as a regex | benign — never compiled/matched |
| `vendored_stdlib/html/parser.py:47,60,76` | `attrfind_tolerant` / `locatetagend` / `locatestarttagend_tolerant` `(?<=['"\t\n\r\f /])…` | parser always calls `.match(rawdata, k)` with k ≥ tag-name-end ≥ 2 — lookbehind never evaluated at pos 0. **Full HTMLParser round-trip (probe E) byte-identical on attribute-laden doc.** `html.parser` itself is only imported by twisted `html.escape` users (`twisted/python/compat.py:32`, `twisted/web/server.py:23`…) — not on the file:// crawl path (parsel/lxml-shim parse the body). |
| `vendored_stdlib/logging/handlers.py:248-276` | `extMatch = r"(?<!\d)\d{4}-…(?!<\d)"` in `TimedRotatingFileHandler.namorize`-adjacent cleanup | **reachable? — verify**: fires only if the gate sets `LOG_FILE` with a TimedRotating handler. Probe D2: subject `"2026-10-22_10-00-00"` (pos-0 lookbehind): CPython span `(0,19)` / impl **None** → **DIVERGES (drops the match)**. Consequence: log-file rotation bookkeeping, not item bytes. |
| `vendored_stdlib/re/__init__.py:54-55` | docstring only | N/A |
| twisted/scrapy/w3lib/parsel/protego/cssselect/jmespath/attr/automat/… | grep `\(\?<[=!]` over both site-packages trees: **0 regex hits** (only the itemadapter strings above) | — |

### Probe evidence
| probe | CPython | impl | verdict |
|---|---|---|---|
| B1 `re.search('(?<=a)','a1').span()` | `(1,1)` | `(0,0)` | **DIVERGES** (repro of finding) |
| B2 `re.findall('(?<!a)','ab')` | `['','']` | `['']` | **DIVERGES** |
| B3 `re.match('(?<!\w)','ab')` | `(0,0)` | `None` | **DIVERGES** (anchored miss) |
| B4 `re.search('(?<=ab)','abc').span()` | `(2,2)` | `(0,0)` | **DIVERGES** (search start-scan reports span 1 pos late) |
| B5 real `attrfind_tolerant` finditer on `' href="x" id=y'`, `'href="x"'`, `'foo=1 bar=2'` | see out | identical | **byte-identical** |
| D2 logging extMatch (pos 0 / mid / prefixed digits) | `(0,19)…` | `None…` (only pos-0 case differs) | pos-0 **DIVERGES** |
| D3 simplified `(?<=ws)[^>]+` on `' href="x"'` | `[(1,9)]` | `[(0,9)]` | **DIVERGES** — impl *over-matches* from 0 |
| E full `html.parser` round-trip (vendored file under impl) | event stream | identical stream | **byte-identical** |

### T1 assessment
Crawl path: **not executed with a pos-0 lookbehind** — the only heavy-regex
consumer on the path (`html.parser`) never matches at 0 and passes the
end-to-end probe. If the gate enables `LOG_FILE` + TimedRotating → miscounted
rotation cleanup (crash-free, no item impact). Severity: low; single
verify-flagged site.

## RF-5 — `lastindex` overcount on repeated alternation

### Sites found
| site | pattern shape | verdict |
|---|---|---|
| `w3lib/_util.py:87` (`attr.lastindex == 1` in `iter_tag_attributes`) | `_attr_re` = named `(?P<name>…)` + ONE optional non-capturing alternation of 3 value groups; alternation is NOT inside a `*`/`+` repeat | **on crawl path** (w3lib.encoding:125,138 charset sniff + w3lib.html:563) — probes C3–C5 on the REAL pattern are **byte-identical**: `href="x"`→2, `href`→1, tag-attr dicts identical |
| `vendored_stdlib/ntpath.py:441-445` (`expandvars` repl uses `m.lastindex`) | `_varpattern` has alternation with groups but no enclosing `*` repeat around the alternation-of-groups | probes C6 (`%FOO%`→1, `$BAR`→2, `%%`→1) **byte-identical**; ntpath only runs on Windows paths — not reached on macOS crawl |
| `vendored_stdlib/re/__init__.py:438` (Scanner.lex) | repeated-alternation lexer shape WOULD hit RF-5 | shadowed (native re has no vendored Scanner; impl `scanner()` raises NotImplementedError, builtins_modules.rs:1946) — **no Scanner users** in either tree (grep `Scanner`: 0 hits) |

### Probe evidence
| probe | CPython | impl | verdict |
|---|---|---|---|
| C1 `re.match('((a)|b)*','ab').lastindex` | `1` | `2` | **DIVERGES** (finding repro — but this shape is NOT in the closure) |
| C2 same `.groups()` | `('b','a')` | `('b','a')` | OK |
| C3/C4 w3lib real `_attr_re` over tag attr strings | dicts | identical | OK |
| C5 `_attr_re` lastindex quoted/valueless | `2 / 1` | `2 / 1` | OK |

### T1 assessment
No closure pattern has "alternation-with-groups inside a repeat" — RF-5 is
**not on the scrapy path**. The one real lastindex branch (`w3lib._util:87`)
was probed against its actual pattern and matches CPython exactly. Severity:
none for T1.

## RF-7 — compile error text/position on repeat shapes

### Sites found
Quick pass over both trees + vendored_stdlib for pattern strings with
consecutive quantifier tokens (`*+`, `*{`, `+{`, `}{`, `){2}{3}` style inside
`r"…"`/`re.compile` context): **0 hits** (only false positives — scrapy's
`http11.py:187` `.{,N}` is a *single* bounded quantifier built by f-string
concat, legal; w3lib/url.py:220 `(\.\./)+` legal). All closure `re.compile`
calls run at import time on valid patterns; nothing exercises the bad-repeat
error text.

### Probe evidence (synthetic only, C7)
| probe | CPython | impl | verdict |
|---|---|---|---|
| `re.compile(r'\Z{2}')` | "nothing to repeat at position 2" | "…position 4" | DIVERGES (text) |
| `re.compile('(a){2}{3}')` | PatternError multiple repeat | **COMPILED** | DIVERGES (accepts) |
| `re.compile('1*+{1,2}')` | PatternError multiple repeat | **COMPILED** | DIVERGES (accepts) |
| `re.compile(r'\11(')` | "invalid group reference 11 at position 1" | "missing ), unterminated subpattern at position 3" | DIVERGES |

### T1 assessment
**none** — requires a caller to compile a malformed pattern; none exists in
the closure. Only risk: spider code in the gate app deliberately compiling a
bad pattern (unlikely, and a crash/TextError would be visible, not silent).

---

## Fix priority for the engine (T1 exposure first)

1. **RF-2 (pattern-method/compiled `count=` kwarg leg only).** The module-level
   splice is fixed; `p.sub(repl, string, count=N)` / `p.subn(...)` / bound
   `.sub` (A4/A5/A7) still over-replace → the only remaining
   **silent-wrong-bytes** class if any future closure or shim code uses it.
   Small, localized fix: kwargs splice in the Pattern-method dispatch
   (`builtins_modules.rs:1915` region, `rest.get(2)` path).
2. **RF-3.** Not currently on the T1 path, but `logging.handlers` pos-0 case is
   live the moment `LOG_FILE` rotation is configured, and lookbehind-at-0 is a
   whole-class correctness hole (B1–B4, D2–D3). Also the probe-B5 result shows
   the bug is narrower than feared — anchoring logic at the lookbehind eval
   position is the fix site in `sre.rs`.
3. **RF-5.** Zero T1 exposure (all real closure patterns byte-identical). Fix
   for fuzz-lane green + generality.
4. **RF-7.** Zero T1 exposure. Error-text alignment work; schedule with the
   fuzz-lane (COMPILE_TEXT family) cleanup.

## Safe workarounds for the T1 agent right now

- **Nothing needs a workaround to go green** — the closure doesn't hit
  RF-2/3/5/7 on the crawl path (proven above, incl. end-to-end html.parser
  probe E byte-identical). Keep going; the lxml shim (02.08-c) stays the
  critical path.
- If a defensive bootstrap is wanted anyway: monkeypatching at gate-app level
  (sitecustomize.py or gate app source, e.g. wrap `Pattern.sub`/`subn` to move
  `count=` positional) is **allowed** — it is gate-app code, not a vendored
  file, so the corpus-only-grows / DEVIATIONS.toml rules are not implicated.
  Do **not** patch `vendored_stdlib/re/__init__.py` or `w3lib/_util.py`:
  vendored-file edits must be declared in `vendored_stdlib/DEVIATIONS.toml`,
  and the `re/__init__.py` wrapper is shadowed by the native module anyway —
  such a patch would be a no-op.
- Clean path remains: fix RF-2 kwarg leg (item 1) before expanding the crawl
  to item pipelines that might use `Pattern.sub(..., count=)`.

## Gaps / not-probed
- protego/robots.txt path not probed separately (its regexes are
  positional-only; no lookbehind/lastindex hits in grep).
- 02.08-c lxml-shim: if it embeds regex-driven HTML parsing in Rust, its own
  exposure is out of scope of this map.
- `ap-run-eval` runs tier-0 with native `re`; a *frozen binary* build of the
  same probes was not run (no cargo invocations allowed — sibling agent).
