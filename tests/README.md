# tests/ — corpus and suite layout

This directory holds the differential-conformance track: Python corpus
cases with pinned expected output, plus pytest suites that run them
against live CPython 3.14 and the antipythony executor.

## Corpus layout

- `tests/corpus/<family>/<case>.py` — one case per file, single statement
  per line where practical (the executor's line tracking is coarse).
- `tests/corpus/<family>/expected/<case>.out|.err|.rc` — pinned stdout,
  stderr, exit code. `.rc` is the exit-code file (not `.exit`).
- Regenerate expectations with the family's `gen_expectations.py`
  (e.g. `tests/t04_ir_verify/gen_expectations.py`), which runs live
  `/opt/homebrew/bin/python3.14` and rewrites the pins. Never hand-edit a
  pin to make a test pass.

## Corpus rules (enforced — see the lints below)

1. **Cites header**: every corpus case starts with
   `# Cites: <clause ids>` citing the spec clauses it pins
   (`S-NN-NN` from docs/spec/, `C-NN` / `D-N` / `A-N` from
   docs/compat-contract.md). Enforced per-suite and globally by
   `tests/corpus_traceability/`, which resolves every cited id against the
   contract + spec pack (stale/typo citations fail the gate).
2. **No salted-hash pins** (S-01-38): never print `hash()` of a literal
   (string/bytes) — CPython salts per process; the value is unobservable.
   `test_no_salted_hash_printed` lints the sources.
3. **Mask, never weaken** (C-33): stderr pins may contain the absolute
   checkout path (CPython 3.14 resolves script paths to absolute in
   tracebacks). Runners normalize with `_mask_root()`, which rewrites any
   absolute prefix before `tests/corpus/...` to `<ROOT>/...` before the
   byte comparison. Never delete a pin line or loosen a message to get
   green — mask paths, keep line numbers and text exact.
4. **Pinned environment** (S-06-15): runners must launch CPython with the
   scrubbed env (PATH/HOME + `LANG=C` only) via `_pinned_env()` /
   `_pinned_env`-style helpers — ambient locale leaks flip
   `io.text_encoding` and corrupt goldens.
5. **Reference interpreter discipline**: anything that runs "CPython"
   must require a real Python 3.14 (`--version` string check), never a
   bare `python3` fallback.

### Bench reports are trend artifacts (robustness-ci 01.05)

`tests/t08_tier1/bench_report.json` is a machine-regenerated timing log,
not a pin: wall-clock numbers are hardware-flaky across runners, so the
report is kept trend-only and is **never** a CI pass/fail gate. It is
listed in `.gitignore` going forward, but the file is already tracked and
committed — untracking it (`git rm --cached`) is deliberately left as a
user call. When the 02.05 tier-1 work lands and wants the >=1.5x
speedup gate, that gate must run on fixed hardware only (self-hosted /
ci-tagged job), comparing ratios rather than absolute times.

## Test tracks

- `tests/corpus/` — byte-pin corpus (rule 1–5 above).
- `tests/t01_parse` … `tests/t16_hybrid` — per-task suites; each cites
  its plan leaf in the header comment.
- `tests/fstrings_corpus/`, `tests/t09_binary/cases/` — alternate layouts
  (family dir with `cases/expected/`, and `.py` + `.stdout/.stderr/.rc`
  triples respectively).

## Running

```
pytest tests/ -q                 # full track
cargo test --workspace           # Rust-side differential tests
cargo run -p xtask -- notices --check   # attribution drift gate
```

A green `pytest` run with `xpassed` entries means an `xfail` marker is
stale: the covered behavior now works. Flip it (remove the marker, keep
the test) rather than leaving it — a stale xfail hides future regressions
of the very behavior it celebrated.
