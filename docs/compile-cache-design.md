# Cross-process compile cache (B4) — design note

Audit reference: `.hermes/audits/2026-11-06-memperf-bughunt.md` finding **B4**
("no cross-process compile cache; the ap-pack cache is write-only").

## Status: half-landed (CLI-side plumbing) — this note records the decision seam

What is implemented on the CLI side (`ap-cli`, `ap-pack::cache`):

- `ap_pack::cache::COMPILED_PAYLOAD` = `"compiled.bin"` — the payload filename
  inside a build-cache entry dir (`<root>/build/<sha256-key>/compiled.bin`).
- `ap_pack::cache::write_compiled_payload(dir, key, body)` — atomic writer
  (temp file + rename under `cache_lock()`). Nothing calls it yet.
- `ap_pack::cache::compiled_hit(dir, key)` — reader/probe. Returns
  `Some(payload_path)` only when the file exists AND its first 64 bytes equal
  the hex `build_key`; a truncated/corrupt/re-keyed file is a silent miss.
- `antipythony run` and `antipythony build` now *probe* the entry dir for a
  key-matched `compiled.bin` (the read-back half B4 asked for). Until the
  deserializer consumer lands, a hit is surfaced under `AP_DEBUG_CACHE=1` and
  execution falls through to tier-0 re-parse, so observable output bytes are
  unchanged (byte-exact gate preserved).

## Format decision (needed before the importer half can land)

The payload is the lowered compile product the importer throws away today:
`ap_ir::Module` (`compile_file` in `crates/ap-run/src/importer.rs` builds it,
`Compiled { module, scopes, ... }`) plus `ap_parse::ScopeInfoModule`.

**Decision: postcard** (`postcard` = serde + varint, no_std-friendly, already
in the tree via wasmtime's dep graph — Cargo.lock pins 1.1.3, so a new direct
dependency adds zero new transitive weight).

Rejected alternatives, with why:

- **rmp-serde (MessagePack)** — human-inspectable-ish, but ~15-25% larger on
  tree-shaped data (map keys repeated per node) and needs an extra schema
  discipline for enums. The cache file is never hand-read; size/speed matter.
- **bincode** — fine, but not currently in Cargo.lock; postcard already is.
- **Custom hand-rolled encoding** — the IR (`ap_ir::Module`,
  `ap_parse::ScopeInfoModule`) is plain data; deriving `Serialize`/
  `Deserialize` on both is mechanical. Only the verifier/golden-dump path
  (`ap-ir/src/dump.rs`) must stay the semantic source of truth, so the
  serde derives live behind a `cache-serial` feature to keep the default
  build's dependency surface unchanged.

## Key/invalidation semantics

Reuse the existing v1 recipe (`build_key`: SHA-256 over
`input NUL lockfile NUL toolchain NUL backend NUL opt_le`, cache.rs module
docs). The embedded-key header makes the payload self-validating; no sidecar
`.key` file.

When the importer consumer lands it should ALSO check `(path, mtime, size)`
of the source file, recorded in the payload body's first fields, mirroring
CPython's `.pyc` invalidation header. The SHA over content already covers
staleness for CLI entry points; the importer keys on path, so the extra
stat-check is mandatory there (a checkout swap reuses the same path).

## Integration points for the remaining half (NOT in this change — owned by
the importer/ap-ir side per the wave plan)

1. `crates/ap-ir` + `crates/ap-parse`: derive serde traits on `Module`,
   `Item`, `Stmt`, `Expr`, `FnDef`, `ClassDef`, `ScopeInfoModule` (behind
   feature `cache-serial`).
2. `crates/ap-run/src/importer.rs::compile_file`: before parse+lower, look up
   `<root>/build/<key(path,lock,toolchain,backend,opt)>/compiled.bin`;
   stat-check header; on hit deserialize and return `Compiled` directly.
   On miss, compile then `write_compiled_payload`.
3. Gate: conformance bytes unchanged (`packaging_differential` 7/0,
   `stdlib_modules_differential` 48/0, corpus non-decreasing) + a new
   differential asserting cold-cache and warm-cache runs are byte-identical.

Pairs with leaf **02.06 (AoT)**: AoT replaces the payload with native object
code under the same entry dir + key; the `compiled.bin` name stays reserved
for the IR form, so AoT should pick `aot.bin` (or version the payload with a
1-byte tag prefix instead — decide at 02.06 time; the embedded-key header
already reserves byte 64+).

## F2 note (same audit family)

`ap-host` now reuses a process-wide `wasmtime::Engine` (`OnceLock`) with an
explicit `run_with_engine(&Engine, ...)` escape hatch; the module-compile
cache for wasm bytes is wasmtime's own `cache` feature (enabled by default in
wasmtime 47), which is the postcard-of-the-postcard question — leave the
wasm-side cache to wasmtime; `compiled.bin` caches the *front-end* (parse +
lower), which wasmtime cannot help with.
