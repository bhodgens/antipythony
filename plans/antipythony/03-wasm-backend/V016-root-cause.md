# Leaf 03.07 V016 root-cause note

## Symptom
`tests/corpus/values/V016-encode-decode-errors.py` (native leg) drops the
`UnicodeEncodeError` line for `"\ud800".encode("utf-8")` — no exception is
raised at all; the encode silently produces U+FFFD bytes (surrB probe:
`b'\xef\xbf\xbd'` where CPython raises `UnicodeEncodeError: 'utf-8' codec
can't encode character '\ud800' ... surrogates not allowed`).

## Root cause (found, NOT fixed in 03.07 — parser layer)
The `\ud800` surrogate is lost at PARSE time, before any encode path runs:
`ruff_python_parser 0.0.4`'s escape decoder maps every `\u` escape in the
surrogate range to U+FFFD:

  ruff_python_parser-0.0.4/src/string.rs:179
  `0xD800..=0xDFFF => Ok(std::char::REPLACEMENT_CHARACTER),`

A lone surrogate is not representable as a Rust `char`, so by the time the
lowered `str_lit` reaches `encode_str` (methods.rs:2294 — which HAS the
correct surrogate rejection arm, proven by `s = chr(0xD800)`-shaped runtime
strings) the character is already U+FFFD and the guard never fires.

Verified: runtime-constructed surrogate strings are impossible too (Rust
`char` cannot hold D800-DFFF), so the only path that can produce a real
lone surrogate in CPython — source text `\ud800` — dies in the vendored
parser dependency. Fixing this needs either an upstream ruff patch /
parser replacement, or a pre-parse escape-preserving shim. That is a
PARSER-layer work item, far outside the 03.07 shared-layer scope and its
file ownership (ap-parse is Plan 01 surface).

## Status for the 03.07 report
- Values corpus: 46/47 native, the 1 fail = V016, root cause ABOVE
  (documented, not a regression; red on trunk for the same reason).
- This is the ONE case in the whole corpus where a runtime that lowers
  does not reach byte-exactness, and it does not involve the wasm backend
  at all. Filed as the named post-03 blocker (parser str-model follow-up,
  queued since 02.03).
