# Plan 00 progress log

## Leaf 00.01 - workspace scaffold - COMPLETE (reviewed, committed)
- Committed d01d477. Ten crates, pinned toolchain (rustc 1.94.0, Homebrew -
  pin is documentation for rustup envs), Apache-2.0 WITH LLVM-exception LICENSE.
- Verified: `cargo build --workspace` exit 0; cargo metadata lists all ten.
- Note: first dispatch agent died without writing files; leaf completed directly.

## Leaf 00.02 - attribution gate - COMPLETE (reviewed, committed 24c97ed)
- deny.toml license policy (Nuitka guard comment), xtask notices generator +
  drift check, THIRD_PARTY_NOTICES (120 deps, 16.9k lines), ruff parser deps in
  ap-parse pinned 0.0.4 (MSRV wall vs rustc 1.94) + get-size2 0.10.1 pin.
- Verified firsthand: build green; notices --check exit 0; tamper->exit 1.

## Leaf 00.03 - compat contract - COMPLETE (committed 0f18715)
- docs/compat-contract.md: 33 clauses C-01..C-33, RFC-2119, each with Verify
  line. Agent live-verified against local CPython 3.14.7.
- OPEN REVIEW ITEMS (for user/main-model):
  1. Local python is 3.14.7, contract pins 3.14.0 - recorded bytes provisional;
     re-verify on real 3.14.0 before first impl gate.
  2. C-01 optional patch-mismatch override env var is agent-invented:
     confirm or strike.
  3. C-25 (source-line echo) asserted at format level only.
  4. C-29 SIGINT->130 verified via self-signal, not real tty; non-POSIX untested.
  5. C-33 CI enforcement (deleted-corpus git check) aspirational - implement in
     CI when Plan 04 release engineering lands.

## Leaf 00.04 - conformance harness - COMPLETE (committed 0f18715)
- ap-conform lib+bin (clap/serde/glob): glob corpus, python3.14 discovery with
  fallback, timeout guard, byte-exact stdout/stderr/exit diff, --json mode.
- Exit codes: 0 all-pass / 1 any-fail / 2 harness error.
- Verified firsthand: self-comparison 2/2 PASS; FAIL path exit 1 with byte
  offset; TIMEOUT path; JSON valid.

## Leaf 00.05 - spec pack - COMPLETE (reviewed; committed e703623)
- docs/spec/01..06 found already written (2,128 lines, 187 S-xx clauses,
  S-01/S-02/S-03/S-04/S-05/S-06 prefixes; written from contract + CPython, not
  impl code - header states provenance).
- Traceability gate: corpus files now cite S-xx ids (fixed this session);
  10-clause spot-check vs live python3.14: 10/10 match (shift, underscores,
  floor div, float repr, str repr, bool arith, None, dict order, chained cmp,
  True==1).
- Spec-05 note: refcount APIs (getrefcount) are corpus-PROHIBITED, not claimed
  absent - consistent with contract D3.

## PLAN 00 COMPLETE.
Next: Plan 01 (frontend/IR) with test leaves T-01..T-04 dispatched from spec
pack in parallel per tests/README.md protocol.
