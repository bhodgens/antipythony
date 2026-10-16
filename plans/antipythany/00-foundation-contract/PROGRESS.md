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

## Leaf 00.02 - attribution gate - DISPATCHED (running)
- deny.toml (deny copyleft), THIRD_PARTY_NOTICES generation + drift-check,
  ruff parser crates added to ap-parse as the live MIT dependency.
- Dispatched to background agent deleg_a3353eb0.

## Queue
- 00.03 (compat contract doc) + 00.04 (conformance harness) can run in parallel
  after 00.02 lands.
- 00.05 (spec pack) last - unblocks the entire test tree.
