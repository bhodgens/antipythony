# 03-sort-dict-json — audit P08+P09+P10

## DISPATCH INSTRUCTION

Audit-only leaf. Do NOT edit implementation code. Do NOT commit.
Parent: plans/primitive-perf/02-parity-audit/orchestrator.md. ~60K.

## Tasks

### T1 — baseline rows
P08 sorted/list.sort, P09 dict build+lookup, P10 json.dumps/loads.

### T2 — our implementations
- sort: ap-run sort arm (builtins.rs sorted / list.sort method) — what
  algorithm? comparison via py_lt dispatch (dispatch cost per compare
  may dominate — distinguish algorithm cost from dispatch cost)
- dict: ap-obj/src/dict.rs — open addressing? insertion order how?
- json: vendored json.py (pure python!) or native? Determine actual
  state — this is likely the biggest gap in the whole audit if it's
  pure-python

### T3 — compare
- CPython: sort → merge sort w/ binary insertion + unsafe_long_compare
  fast paths; dict → open addressing + cached hashes; json → C
  scanner/encoder
- Rust SOTA: std sort_unstable (pdqsort), hashbrown, serde_json
  (5-10x CPython json typical)
- Per-primitive verdicts

### T4 — findings doc
`audits/sort-dict-json.md`. For json specifically: if pure-python,
write the concrete native-json leaf sketch (parser + encoder in Rust,
serde_json-class, byte-exact output including separators/sort_keys/
ensure_ascii semantics) — this will likely be the #1 dispatch
recommendation to master.

## Self-verification checklist
- [ ] every P08/P09/P10 row explained
- [ ] json leaf sketch present if pure-python confirmed
- [ ] sort analysis separates compare-dispatch cost from sort algorithm

## Review checklist (orchestrator)
- [ ] json verdict grounded in actual code (grep the import path)
- [ ] dict hash-cache story checked (do we cache str hashes? we do —
      confirm it survives lookups)
