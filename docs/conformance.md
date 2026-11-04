# Conformance Methodology

Antipythony's exit gate is not "passes a test suite". It is **byte-exact
differential conformance against CPython 3.14**: for the same script, our
output bytes, error bytes, and exit code must equal CPython's. This
document explains the machinery that enforces that.

## The reference interpreter

- Contract pin: CPython 3.14.0 (`docs/compat-contract.md`).
- Local reference: `python3.14` (Homebrew Cellar on macOS; suites hardcode
  `/opt/homebrew/bin/python3.14` via `REF_PY`). A recorded expectation
  generated on 3.14.7 is provisional until re-verified on 3.14.0 - an
  open review item.
- Reference runs use a scrubbed environment (`_pinned_env()`:
  PATH/HOME + `LANG=C` only). Ambient locale leaks flip
  `io.text_encoding` and corrupt goldens.

## ap-conform

The differential runner (`crates/ap-conform`):

```
cargo run -p ap-conform -- run --corpus 'tests/corpus/values/*.py' --impl <cmd> [--filter <pat>] [--tier 0|adaptive] [--json]
```

For each corpus script it runs the reference (`python3.14`) and the impl
command, captures stdout, stderr, and exit code, byte-diffs all three,
and emits a per-case pass/fail table. `--tier` is forwarded verbatim to
the impl (`<impl> ... --tier <mode> <file>`); the reference never
receives it. Milestone metric: pass rate over a corpus slice. The failure
taxonomy (missing-feature vs wrong-bytes) is the work queue: wrong-bytes
are conformance bugs; missing-feature items are queue entries.

## The corpus

`tests/corpus/<family>/<case>.py` with pinned
`expected/<case>.out|.err|.rc` (`.rc` is the exit-code file). Families:
hello, types, fstrings, values, classes, exceptions, gc, stdlib,
threading, tier1-transparency. Current: 150/197 green.

Rules (enforced; see `tests/README.md`):

1. **Cites header.** Every case starts with `# Cites: <clause ids>` -
   `S-NN-NN` from `docs/spec/`, `C-NN`/`D-N`/`A-N` from the compat
   contract. `tests/corpus_traceability/` resolves every citation against
   the spec pack; a stale or typo'd citation fails the gate.
2. **No salted-hash pins.** Never print `hash()` of a str/bytes literal -
   CPython salts per process; a lint enforces this.
3. **Mask, never weaken.** Stderr pins may contain the absolute checkout
   path; runners normalize it to `<ROOT>/...` before comparing. Deleting
   a pin line or loosening a message to get green is forbidden.
4. **Corpus only grows.** No case is weakened or deleted to pass a gate.
   Env-dependent goldens are regenerated from the reference with a
   documented correction note.
5. **Generator scripts live in `tools/` subdirs**, never beside case
   files (a generator inside the glob range corrupts pass-rate counts).

Expectations regenerate via the family's `gen_expectations.py`, which runs
live python3.14 and rewrites the pins. Never hand-edit a pin.

## Separation of duties

- The **spec pack** (`docs/spec/01..06`) is the single semantics source.
  Clause ids are permanent; corrections are numbered amendments (A-1..A-6
  applied so far), never renumberings.
- **Test leaves (T-01..T-16)** are authored by different agents, in a
  separate pass, ONLY from spec + master. Test agents get a firewall: no
  `crates/` reads, no git history. Probing the reference interpreter to
  pin expected bytes is allowed - that is the spec made executable.
- **Suites are authored xfail-first** against per-leaf testability bins
  (`ap-parse-check`, `ap-ir-dump`, `ap-verify-dump`, `ap-run-eval`,
  `ap-obj-eval`); they flip to real runs automatically when the bin
  lands. Tests live in files strictly separate from implementation - the
  tests are that module's contract and survive lost source.
- A leaf's completion gate is its named test leaf, green, run by the
  orchestrator. Before accepting a large implementation leaf, the
  orchestrator also runs 5-10 fresh five-line probes chosen from the
  spec's user-visible contract (not the leaf's own probe set) and diffs
  them against python3.14. Fresh smoke has caught real bugs while the
  leaf's own suite was green.

## Tier transparency

Tier-1 (JIT) correctness rule: any observable difference from tier-0 is a
bug. The conformance suite therefore runs in both forced tier-0 and
adaptive modes and requires identical bytes (T-08).

## Divergences

Divergences are only legitimate when written down in
`docs/compat-contract.md` or a leaf's documented gap list. Current
accepted examples:

- `__del__` runs but timing is not guaranteed (tracing GC, no refcount).
- The `re` engine raises a step-limit error where CPython would hang on
  catastrophic backtracking (no corpus/probe hits it).
- Bytes patterns in `re` decode latin-1 into the str path (v0 gap,
  documented).
- Arbitrary-precision ints everywhere; `range()` bigint clamp documented.
