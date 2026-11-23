# NEXT-STRFIX — tier1-strfix leaf (2026-11-23)

## Leaf: str.splitlines(keepends=True) terminator dropping + bytes/bytearray.splitlines
Branch `tier1-strfix` off main @ 3bafad9; worktree `~/.git/worktrees/leaf-strfix`.
Commit **c04964a** (explicit pathspecs).

## Root causes
1. `pystr.splitlines` (crates/ap-run/src/builtins_modules.rs) accumulated
   lines into a `String` and NEVER appended the terminator — keepends arg
   was ignored entirely (both modes dropped terminators).
2. `bytes.splitlines` / `bytearray.splitlines` did not exist (AttributeError):
   no pybytes arm, not in `builtin_value_method_exists`, not in the bytearray
   attr-surface name list, no `call_bytearray_method` arm.

## Fixes (all in crates/ap-run/src/builtins_modules.rs unless noted)
- pystr.splitlines: byte-offset slicing rewrite. keepends=True emits
  `[line_start..line_end]` (terminator included); `\r\n` consumed as ONE
  terminator; trailing chunk emitted only when non-empty
  (`"" -> []`, `"a\n" -> ['a']` / `['a\n']`). Positional + keyword keepends,
  truthy() semantics (keepends=0/'' -> False).
- pybytes.splitlines (NEW): ONLY `\n` `\r` `\r\n` are bytes boundaries
  (`\v \f \x1c \x85 ...` are str-only — CPython parity pinned in V066).
- bytearray.splitlines: delegates to the pybytes arm over a buffer snapshot,
  re-wraps items as bytearray. En-route: `rsplit` + `splitlines` added to the
  bytearray attr-surface marker list (rsplit was missing there too).
- crates/ap-obj/src/str_repr.rs: `\u2028`/`\u2029` (Zl/Zp, not
  unicodedata-printable) now escape in repr() as `\u2028`/`\u2029` — found by
  the differential probe, CPython-exact.

## Gates (all measured on the worktree, release build)
- tests/t5-probe/splitlines_keepends_repro.py: green + BYTE-IDENTICAL vs
  /opt/homebrew/bin/python3.14.
- tests/t5-probe/splitlines_diff_probe.py (NEW, 71 print lines): every
  terminator class x keepends, bytes-vs-str boundary sets, bytearray typing,
  join round-trips — byte-identical vs CPython 3.14.7.
- Corpus pins: tests/corpus/values/V065-splitlines-keepends-terminators.py +
  V066-bytes-bytearray-splitlines.py with expected/{stdout,stderr,exit,meta}
  recorded under python3.14 (meta via tests/tools/_gen_meta.py values).
  ap + CPython stdout diff: identical.
- cargo test -p ap-obj --release: 71 passed / 0 failed.
  cargo test -p ap-run --release: 208 passed / 0 failed / 1 ignored.
- ap-conform stdlib slice: 48 cases, 48 passed, 0 failed (impl =
  target-strfix/release/ap-run-eval).

## GOTCHAS hit this leaf (for the next fix-leaf)
- The fix_pass_differential suites spawn `ap-run-eval` from
  `$CARGO_TARGET_DIR/debug/` (DEBUG path even under --release). A
  release-only build leaves it missing -> "spawn fenced cmd: No such file or
  directory" panics in bugb_* tests. `cargo build -p ap-cli --bins` (debug)
  fixes it. Pre-existing trap, not caused by this leaf.
- Running `tests/tools/_gen_meta.py` with NO family args rewrites EVERY
  family's .meta date field (~250 files of churn). Always pass the family:
  `_gen_meta.py values`.

## Not done / follow-ups
- str.splitlines maxsplit: CPython 3.14 splitlines has NO maxsplit — nothing
  to do. keepends keyword-only misuse errors (e.g. keepends=2 positional)
  match CPython by truthiness, not by TypeError surface — untested edge,
  filed here only.
- bytes/bytearray `.splitlines()` via type-object unbound call
  (bytes.splitlines(b'...')) — not probed this leaf; instance form is the
  pinned surface.
