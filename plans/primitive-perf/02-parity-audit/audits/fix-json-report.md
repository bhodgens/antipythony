# Fix-leaf report — ppfix-json (P10 native json.loads / json.dumps)

Leaf: `ppfix-json` (W6b, tracker `TRACKER-PRIMITIVE-PERF.md`).
Worktree: `/Users/caimlas/.git/worktrees/leaf-ppfix-json`
(branch `tier1-ppfix-json`, forked @ main `8a41c4f`).
CPython ref: `/opt/homebrew/bin/python3.14` (3.14.7).
Scope held: the json region of `crates/ap-run/src/builtins_modules.rs`
(JsonParser + json_loads/json_dumps/dump_value/dump_seq/dump_string) +
new gate `tests/bench/primitive/fix_json.py` + this report. No new
crate deps (memchr NOT adopted — hand-rolled u8 scan loops instead,
per the leaf constraint).

## Commits (explicit pathspecs)

| commit | what |
|---|---|
| `22d1aaf` | test: `tests/bench/primitive/fix_json.py` differential gate (127 cases) |
| `e23528b` | perf increment 1: bulk-scan loads + escape-table dumps (+5 correctness fixes) |
| `f43f93a` | perf increment 2: object-key memo + escape-free string fast path |

## Gate results (all re-run on the final increment)

| gate | result |
|---|---|
| `fix_json.py` differential vs CPython 3.14.7 | **127/127 GREEN** |
| `ap-conform run --corpus tests/corpus/values/*.py` | **50/50 PASS** |
| `ap-conform run --corpus tests/corpus/stdlib/s0*.py` | **48/48 PASS** (incl. s008/s009 json rows, which were RED at gate-authoring time — see below) |
| `cargo test -p ap-run --release` | **208 passed / 0 failed / 1 ignored** (the 2 initial `bugb_*` failures were the KNOWN debug→release symlink gotcha; symlinked `debug/ap-run-eval` + `debug/ap-run-probe` → release per prior-leaf discipline, suite then green) |
| bench slice | see table below |

## Bench (run_bench.py --only P10, 3 runs, loadavg < 10)

| row | before | after (3 runs) | verdict |
|---|---|---|---|
| json.dumps @48KiB | 0.92–0.97x | **1.01x / 1.02x / 1.03x** | **PASS — leaf dumps target MET** |
| json.dumps @1MiB | 1.02–1.05x | **1.02x / 1.03x / 1.13x** | PASS |
| json.loads @48KiB | 0.23–0.24x | **0.31x / 0.32x / 0.34x** | FAIL vs ≥1.0x target (see honest gaps) |
| json.loads @1MiB | 0.17–0.18x | **0.20x / 0.21x / 0.22x** | FAIL vs ≥1.0x target (see honest gaps) |

Raw JSONs: `/tmp/ppfix_json_p10.json`, `/tmp/ppfix_json_p10b.json`,
`/tmp/ppfix_json_p10c.json` (+ baseline
`/tmp/ppfix_json_baseline.json`, pre-fix).

## What changed in the parser (json.loads)

1. **Bulk string scanning** (`parse_string`): plain-u8 scan to the
   closing quote with a backslash/control classifier; escape-free
   spans slice-copied whole; the common escape-free case now returns
   the doc slice directly (zero intermediate buffer). `\\uXXXX` decode
   via a `HEX_VAL [u8;256]` nibble table (4 lookups + shifts,
   `from_str_radix` gone).
2. **Bulk number parsing** (`parse_number`): single-span scan then
   decode; i64 fast path (≤18 digits pure accumulate, 19–20-digit
   `try_parse`, Big only past i64); float via one
   `str::parse::<f64>` on the span.
3. **Object-key memo** (`parse_object`): CPython's `_json` memo-dict
   equivalent — recurring keys re-serve the SAME `PyString` handle
   from a per-loads `HashMap`, keeping `cached_hash` warm and dict
   probes on the identical-Arc eq fast path. `pairs` vec only
   materializes when `object_pairs_hook=` is set.
4. **Pre-sized buffers**: array/object element `Vec::with_capacity`
   from a depth-aware forward precount (capped 4096).
5. **Bulk whitespace skip** with ptr-advance.

Isolation probes (best-of-5, quiet window) — the new kernels vs CPython:

- single 1 MB escape-free string: **1.42x FASTER than CPython**
- 1.1 MB / 30k-string array: **1.46x FASTER**
- 200k-int flat array: **2.15x FASTER**
- 50k-key object: **2.1x SLOWER** (container-mint bound, see gaps)

## What changed in the encoder (json.dumps)

1. **Pre-sized output** `String::with_capacity(json_out_hint(v))` —
   one-walk tree-size heuristic (was: empty + ~20 growth reallocs on
   a 1 MiB tree).
2. **`DUMP_CLASS [u8;256]` escape classifier** + bulk verbatim span
   copies (`esc_span_len`) + direct `\uXXXX` writer — the `format!`
   per-escape machinery is gone. `sort_keys` / `ensure_ascii` /
   `separators` / `indent` semantics byte-identical (gate F rows).
   One regression caught by the stdlib gate during development: the
   `ensure_ascii=False` non-ASCII arm initially escaped instead of
   pushing verbatim — s008/s009 caught it at first differing byte,
   fixed, 48/48 green after.

## Correctness fixes landed WITH the perf work

The new `fix_json.py` gate measured **11 divergences vs CPython at
baseline** (pre-optimization). All fixed by the rewrite, gated green:

- `Invalid \uXXXX escape` position: backslash+1 (CPython shape; was
  after the 4-hex scan)
- leading-zero numbers (`01`, `[01]`) now rejected (Extra data /
  Expecting ',' delimiter like CPython; were silently accepted)
- `1e` / `1e+` / `1.` now stop at the int span → "Extra data" (were
  accepted as valid numbers)
- Unterminated string position = the **opening quote** (was EOF)
- unescaped control bytes < 0x20 → "Invalid control character at"
  (were accepted verbatim)

Column-parity on multi-byte docs verified: char pos = BYTE pos,
column = CHAR pos (`line_col` semantics already exact; probed with
CJK before/after).

## Honest gaps (two focused attempts spent on each, per leaf budget)

1. **loads ≥1.0x CPython NOT MET on the bench shapes (0.20–0.34x).**
   Root cause isolated, NOT in this leaf's file ownership: the
   per-container mint floor. The P10 probe shape is record-heavy
   (16000 records = 16k dicts + 48k lists + ~100k string mints);
   every `PyDict::new` / `PyList::from_vec` pays `Gc::new` +
   `gc_register_pub` (Mutex-locked registry HashMap insert +
   process-global threshold ladder) ≈ 1 µs/container vs CPython's
   ~0.15 µs — that alone accounts for the whole residual. Evidence:
   parse KERNELS (strings, ints, the scan+decode itself) measure
   1.4–2.2x FASTER than CPython in isolation (see probes above);
   dict-mint-bound shapes measure 2.1x slower. Filed follow-up:
   bulk-allocation API or registry batching for json-shaped container
   construction (ap-obj/ap-gc owners), NOT another builtins_modules
   pass — the json region itself is at kernel parity.
2. **NaN/Infinity loads**: `json.loads('[NaN, Infinity]')` raises in
   ours (CPython's own decoder accepts the literals — itself an
   RFC-8259 extension). The fix_json gate gates dumps' byte output
   for NaN/Infinity but skips the roundtrip leg (`_rt: False`,
   documented in the gate). Decoder feature gap, filed, out of the
   perf-leaf scope.
3. **Pre-existing divergences deliberately left untouched** (not
   gated green here, out of scope): error surface strictness beyond
   the 5 fixes above (e.g. `Infinity` literal in loads, lone low
   surrogate `\udc00` decode shape), sort_keys non-str-key ordering
   (type_name sort vs CPython TypeError — noted in the audit, not a
   P10 perf row).

## Tool notes / gotchas hit

- The pre-existing `ESC_CHARS`/`ESC_DECODE` sketch I initially wrote
  was unused after the design settled on HEX_VAL + direct char pushes
  — removed before commit (zero-warning build).
- The bench launcher's `--only` slice output buckets P10's 71558 B /
  1080957 B payloads into the 48KiB / 1MiB labels (known harness
  behavior, docs/primitive-perf-report.md §Timing).
- Machine was heavily loaded (loadavg up to 33, sibling ppfix lanes
  running) during early bench attempts — numbers were garbage;
  final three runs taken in a quiet window (loadavg 8–9) per the
  machine load policy.
