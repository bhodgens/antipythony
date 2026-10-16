THIRD_PARTY_NOTICES generation for antipythany
==============================================

Command surface (the supported path)
------------------------------------

    cargo run -p xtask -- notices          # regenerate THIRD_PARTY_NOTICES
    cargo run -p xtask -- notices --check  # CI gate: exit 1 on drift

Output is deterministic: entries are sorted by (crate name, version) and the
template is fixed, so regenerating twice produces byte-identical output. The
CI gate regenerates in memory, diffs against the committed file, and fails
with `NOTICES DRIFT` if they differ.

cargo-about (installed but not load-bearing)
--------------------------------------------

cargo-about 0.9.2 is installed locally (`cargo install cargo-about --locked
--features cli`). It was evaluated as the generator and passed over in favor
of the xtask implementation because:

- cargo-about needs an about.toml + handlebars template per crate and pulls
  license text from its own klasky-suite config; output depends on its
  version, making the committed file less stable across toolchain updates.
- The xtask route has zero dependencies beyond cargo_metadata (already in
  the lockfile graph), so the gate builds everywhere the workspace builds,
  including constrained CI images.
- cargo-deny (`deny.toml`) still covers license *policy*; the xtask covers
  notices *freshness*. The two are complementary, not redundant.

If the project later wants cargo-about's per-clause attribution or
`--cluster` features, add `about.toml` + `about.hbs` and wire
`cargo about generate -o THIRD_PARTY_NOTICES about.hbs` into a compare step;
the xtask `--check` semantics transfer directly.

License policy (deny.toml)
--------------------------

Allowed: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-3-Clause,
BSD-2-Clause, ISC, Zlib, Unicode-3.0, CDLA-Permissive-2.0.

Denied: all GPL/AGPL/LGPL variants, MPL (file-level copyleft must be a
reviewed, deliberate addition), CC-BY-SA, SSPL, EUPL. Documented dev-only
exceptions go in `[[licenses.exceptions]]` in deny.toml with a comment
naming the leaf that justified them.

Nuitka note: Nuitka is AGPL-3.0 and is used (if ever) strictly as a
subprocess tool for comparison builds. It is NOT a Cargo dependency and must
never appear in Cargo.lock or in this notice file. A `nuitka` entry anywhere
in the Rust dependency graph is a bug; stop and investigate.

Known-good data points (as of leaf 00.02)
-----------------------------------------

- The ruff component crates (ruff_python_parser, ruff_python_ast,
  ruff_text_size, ruff_python_trivia, ruff_source_file) do not ship a
  LICENSE file in their crates.io archive. xtask embeds the MIT text from
  astral-sh/ruff for them (see crates/xtask/src/license_fallbacks.rs); the
  on-disk LICENSE file wins over the table if a future version ships one.
- get-size2 is pinned to =0.10.1 and ruff crates to 0.0.4 in
  crates/ap-parse/Cargo.toml: ruff 0.0.5+ requires rustc 1.95+ (we pin
  1.94.0), and get-size2 0.10.2+ bumped compact_str to 0.10 while ruff
  0.0.x pins compact_str 0.9. Revisit both pins when the toolchain moves
  past 1.94.
