# NEXT-LINECACHE — session record (leaf tier1-linecache, 2026-11-22/23)

Branch: `tier1-linecache` (worktree `~/.git/worktrees/leaf-linecache`,
base main @ 663c216). Machine: M2 Ultra, CARGO_TARGET_DIR=target-linecache,
CARGO_BUILD_JOBS=4, one heavy lane throughout.

## What landed

AOT source embedding (02.06 follow-up / 02.08 audit row 4): the vendored_stdlib
.py sources (465 files / 7,105,814 bytes; pydoc_data + _pyrepl excluded) are
compiled into ap-run as a static `include_bytes!` store
(`crates/ap-run/src/embedded_stdlib/{data.rs, mod}`), and the importer's file
finder + linecache registry consult it ONLY when the build tree is absent
(`embedded_store_active()`: vendored dir missing or != the build-time tree).
Native registry stays first-face; disk wins when present; repo-run behavior is
byte-identical to pre-leaf. __file__ labels keep the CPython-shaped
`vendored_stdlib/<rel>.py` form. Regenerate via `xtask/gen_embedded_stdlib.py`
(deterministic: dotted keys, byte-order sorted, __init__ folded, package face
wins over sibling module face).

Commits: 6d59fd1 (store + importer fallback + linecache arm),
8f66224 (store-test fix: empty vendored __init__ faces are legal — urllib,
email.mime are genuinely empty on disk too), d809a06 (gate doc:
docs/aot-stdlib-embed.md).

## Gates (all executed, transcript in docs/aot-stdlib-embed.md)

1. Binary-mode stdlib slice: **46/48** — re-measured BEFORE the leaf on this
   tree: also 46/48. The audit's binary-only s021_linecache gap was ALREADY
   closed on main by the 0206-embed `ap_aot_script_path` + `set_embedded_source`
   seam (linecache reads the embedded PROGRAM source); what this leaf adds is
   the VENDORED-STDLIB face: standalone import chains (enum/warnings/inspect/
   dis/opcode/...) + linecache reads of vendored `__file__`s resolve without
   the build tree (verified: baseline binary FAILS the same probes with
   ModuleNotFoundError; leaf binary byte-exact). The 2 slice fails are s013
   functools + s023 ast.unparse — the audit's DOCUMENTED interpreter-mode
   surface gaps, out of embedding scope. No row was faked green.
2. T-09 30-case slice through the BINARY: 30/30 inside 90 passed/0 failed;
   hello + c001 byte-exact vs python3.14; startup 0.87x (min-of-7), soft gate
   intact.
3. cargo test --workspace --release -j4: 122 suites, 663 passed / 0 failed /
   1 ignored, RC=0.
4. Size delta (honest): hello-world binary 0-byte delta (store dead-strips
   when unreachable); enum-chain binary +9.8MB over baseline (store reachable).
   MUSL stays report-only (T-09 GAP-2).

## En-route notes

- The workspace gate tripped the DOCUMENTED fresh-worktree harness gotchas,
  one at a time: (a) ast_getattr_fence needs `ap-run-probe` at the baked
  repo-root `target/release/` path; (b) build_ext_gate needs the DEBUG
  antipythony at `$CARGO_TARGET_DIR/debug/`; (c) hybrid_gate needs release
  `ap-host` + `ap-aot-crt-hybrid` pre-built. All built before the final green
  lane; consider pre-building these in future leaf setup steps.
- `ap-conform --impl-binary` drives the same build+run+diff triple the manual
  slice used; the manual loop was preferred for the before/after A/B because
  it survives the session's flaky backgrounding.

## Next steps

- Plan 02 remaining per the audit: iter(<genexpr>) wedge (blocks s030/s044),
  OUT-face spec amendment, the ~16 named surface gaps (s013/s023 included),
  T121 quiet-window re-verify.
- Plan 04 packaging can now ship the store instead of the vendored dir (the
  wheel no longer needs the tree at runtime for embedded binaries).
