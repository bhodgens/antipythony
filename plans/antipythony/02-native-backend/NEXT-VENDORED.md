# NEXT-VENDORED — vendored stdlib backfill: colorsys + urllib.request parse_http_list

Date: 2026-11-23. Worktree: `~/.git/worktrees/leaf-vendored` (branch
`tier1-vendored`, off main `3bafad9`, ff-advanced to `063b7ee` before commit).
Target dir: `target-vendored` (CARGO_BUILD_JOBS=4; one heavy cargo lane held;
log: `~/.git/worktrees/leaf-vendored/gate-stdlib-diff.log` equivalent — suite
output in-session, 48/48).

## Commit

- `8d52698` vendored-stdlib backfill: colorsys.py (pristine 3.14.7) +
  urllib.request parse_http_list/parse_keqv_list (verbatim defs).
  Pathspecs: vendored_stdlib/colorsys.py, vendored_stdlib/urllib/request.py,
  vendored_stdlib/DEVIATIONS.toml. 3 files, +235/-3.

## What landed

1. **vendored_stdlib/colorsys.py** — byte-identical copy of CPython 3.14.7
   Lib/colorsys.py (sha256 65e3dfbf7bad61d4d7d7731a69dd7e75a347fd350d91327a51010a94e6fd2f1d,
   confirmed with cmp + shasum vs
   /opt/homebrew/Cellar/python@3.14/3.14.7/.../lib/python3.14/colorsys.py).
   The staged t3chat/colorsys.py workaround (leaf-t3) was byte-identical, so
   this formalizes it. PSF-2.0 provenance, same as every other vendored file.
2. **vendored_stdlib/urllib/request.py** — `parse_http_list` +
   `parse_keqv_list` appended verbatim from CPython 3.14.7 (defs byte-extracted
   and diffed equal vs the upstream file); `__all__` extended; header comment
   updated. httpx/_auth.py:9 `from urllib.request import parse_http_list` now
   resolves.
3. **vendored_stdlib/DEVIATIONS.toml** — colorsys.py pristine entry (sha-pinned)
   + urllib/request.py deviated_sha256 refresh
   (8b21c296... → 07ffa8bedf9056c09dfe7e5804320540ac5e1fe8faa5c9f11e528ecdfd140ebe)
   with the T3-wave note.

## Gates (executed, not claimed)

- colorsys byte-identity: cmp/sha vs Homebrew CPython 3.14.7 — IDENTICAL.
- colorsys behavior probe (probe_cs.py, 729 rows: 9^3 rgb grid × hls/hsv/yiq
  forward+round-trip reprs via json.dumps): byte-identical vs python3.14.
- parse_http_list/parse_keqv_list probe (probe_phl.py, 9 vectors incl.
  quoted commas, escaped quotes, unclosed quotes, empty input): stdout
  byte-identical vs python3.14.
- `cargo test -p ap-cli --test stdlib_modules_differential`:
  **48 passed / 0 failed** (unchanged).
- `cargo run -p xtask -- stdlib-provenance --check`: delta-clean — base
  (stash-check) 13 problems incl. "colorsys.py has no DEVIATIONS.toml entry";
  after my changes 12 problems, ALL pre-existing at the base commit
  (STALE enum.py/typing.py/_ast.py/annotationlib.py/collections.py/functools.py/
  inspect.py/types.py deviated shas + UNDECLARED tokenize.py drift — owned by
  commits 9711381/7e134c9 waves that touched vendored bytes without
  re-declaring; NOT this leaf's files, left for the provenance-remediation row).
- httpx import-face: `from urllib.request import parse_http_list` +
  `import httpx` now proceed PAST the S3 wall of NEXT-T3.md (previously
  ImportError).

## New walls recorded (do NOT fix in vendored leaves)

- **S3 next wall** — httpx/_models.py:1243
  `class _CookieCompatRequest(urllib.request.Request)`: vendored request.py
  (trimmed copy, no socket/email/http.client) has no `Request` class.
  `AttributeError: 'module' object has no attribute 'Request'`. FULL import of
  httpx still blocked here; FURTHER vendored gap (Request class = urllib
  machinery surface), next vendored leaf or interp decision.
- **S5 next wall** — `from rich.panel import Panel` with colorsys present
  proceeds past F-VENDORED-2 into rich/console.py:80 class-level
  `sys.__stdin__.fileno()` → `io.UnsupportedOperation: fileno` escaping
  through vendored typing.py NamedTupleMeta:3077 → enum.py _proto_member.
  Same F-INTERP-1 fileno wall as NEXT-T3.md recorded (bytes identical);
  rich itself untouched per scope.

## Probes (left in place, not committed)

- t3chat/probe_phl.py, t3chat/probe_cs.py, t3chat/probe_s3.py,
  t3chat/probe_s5.py (main checkout, untracked) — standing repro drivers.
- t3chat/deps/pydantic_core.so built via `antipythony build-ext` (untracked).

## Machine policy note

leaf-fileno's release lane ran concurrently 12:59PM; my first background
cargo-test attempt died in the race — re-ran in foreground after the sibling
lane drained. One heavy lane at a time was honored.
