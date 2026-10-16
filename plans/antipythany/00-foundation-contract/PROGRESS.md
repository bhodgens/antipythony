# Plan 00 progress log

## Leaf 00.01 - workspace scaffold - COMPLETE (reviewed, committed)
- Committed: 4f3d033..HEAD "feat(00.01): workspace scaffold - ten crates, pinned
  toolchain, Apache-2.0 WITH LLVM-exception".
- Verified: `cargo build --workspace` exit 0; `cargo metadata --no-deps` lists all
  ten crates (ap-parse, ap-ir, ap-obj, ap-gc, ap-run, ap-codegen-native,
  ap-codegen-wasm, ap-host, ap-cli, ap-conform); Cargo.lock committed;
  LICENSE = Apache-2.0 + LLVM-exception verbatim; rustc 1.94.0 pinned in
  rust-toolchain.toml (Homebrew cargo ignores the pin - documented in-file).
- Note: first dispatch agent died without writing files (cause unknown). Leaf was
  completed directly by the orchestrator; directories it had half-created were
  reused.

## Leaf 00.02 - attribution gate - COMPLETE (reviewed, committed 24c97ed)
- Verified firsthand: build green; `xtask notices --check` exit 0 (120 deps,
  up to date); drift-check tamper test documented by agent + check re-run clean;
  THIRD_PARTY_NOTICES 16,939 lines with ruff MIT text present; deny.toml carries
  Nuitka guard comment.
- Route: xtask (not cargo-about); ruff crates pinned 0.0.4 due to MSRV wall
  (1.95-1.97 MSRV vs our rustc 1.94.0) + get-size2 0.10.1 pin for compact_str
  split - both documented in docs/attribution.md; revisit at toolchain bump.

## Leaf 00.03 (compat contract) + 00.04 (conformance harness) - COMPLETE
- 00.03: docs/compat-contract.md, 33 clauses C-01..C-33, RFC-2119 force, each
  with a Verify line. Agent live-verified claims against local CPython 3.14.7
  (MemoryError on 1<<(2**62); exit-0-with-stderr-report for __del__/atexit
  raises). OPEN REVIEW ITEMS: local python is 3.14.7 not pinned 3.14.0 ->
  recorded bytes provisional (C-01); C-01 patch-mismatch override env var is
  agent-invented - confirm or strike; C-25 source-line echo format-level only;
  C-29 SIGINT via self-signal not real tty; C-33 CI enforcement aspirational.
- 00.04: ap-conform harness (lib+bin, clap/serde/glob), 2 seed corpus scripts.
  Verified firsthand: build green, self-comparison 2/2 PASS exit 0 vs
  python3.14 (3.14.7), FAIL path exits 1 with byte-offset diff, TIMEOUT path
  works, --json valid. Committed 0f18715.
- NOTE: docs/spec/01-06 already exist in the repo (agent found them present).
  00.05 reduces to: review spec pack vs contract clause ids + traceability gate.

## Remaining in Plan 00
- 00.05: main-model review of spec pack vs compat-contract, traceability check
  (every corpus case traceable to a clause id), then Plan 00 COMPLETE.
- Reviewer actions: strike/confirm C-01 env var; schedule 3.14.0 re-verification.
