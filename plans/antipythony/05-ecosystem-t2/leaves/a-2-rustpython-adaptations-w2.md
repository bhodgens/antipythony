# Leaf A-2: RustPython leaf-donor adaptations, wave 2

## Source
docs/rustpython-leaf-donor-inventory.md top-10 items 7, 9, 10 + gap-table
"partial" rows that are leaf-level portable. Donor: /tmp/rustpython-study
(MIT). Policy: docs/rustpython-donor-policy.md (leaf-level only; attribution
headers "adapted from RustPython (MIT), <file:line>"; xtask notices regen).

UNBLOCK CONDITION: leaf A-1 must land first - both waves write
crates/ap-run/src/builtins_modules.rs. Dispatch A-2 only after A-1 is
committed by the orchestrator.

## Items
1. hashlib algorithm expansion (inventory item 7): add sha3_224/256/384/512,
   blake2b/blake2s, shake_128/256 via RustCrypto crates (crate-LOCAL deps in
   crates/ap-run/Cargo.toml); algorithms_available/algorithms_guaranteed
   sets. DO NOT port their PyHasher object model (needs-redesign) - keep our
   object surface, port the algorithm layer only. Fix our hexdigest
   callability (known T2 gap) using the established BuiltinFn-marker
   pattern. Donor ref: crates/stdlib/src/hashlib.rs, hmac :1191.
2. zlib one-shot (inventory item 9): real adler32/crc32/compress/decompress
   via flate2 (bodies at donor crates/stdlib/src/zlib.rs:63-295, :98/:120).
   Streaming compressobj/decompressobj objects stay needs-redesign - out of
   scope. flate2 = crate-LOCAL dep. Our current zlib is an import-level stub
   (docs/t1-gate.md known gap); upgrade the surface, keep import behavior.
3. os fd family (inventory item 10): os.dup/dup2 (donor posix.rs:2326/:2341),
   os.fstat + dir_fd/follow_symlinks on stat/lstat (os.rs:1585/:1601/:1611).
4. os.uname structseq (donor posix.rs:1772) replacing the platform.py shim
   path (builtins_modules.rs ~:7612).
5. os.path.realpath symlink resolution: pure string walk, posixpath
   semantics (donor posixpath implementation; ours is str-only at
   builtins_modules.rs ~:7134).
6. select unsupported-error types (trivial rating, inventory adaptation
   costs): kqueue/epoll/poll request on a platform without them raises the
   same error type/text CPython raises - probe python3.14 first, the
   inventory's paraphrase is not evidence.

## Gates
- cargo test --workspace 0 failed; pytest tests/ non-decreasing; corpus
  non-decreasing; xtask notices --check exit 0 (RustCrypto + flate2 are new
  deps - notices regen is mandatory).
- New differential cases in crates/ap-cli/tests/stdlib_modules_differential.rs:
  sha3/blake2/shake digests vs python3.14, zlib compress/decompress roundtrip
  + adler32/crc32, os.dup/dup2 fd shuffling, os.fstat on a real fd,
  os.uname field shape (skip non-macOS divergence), realpath through a real
  symlink chain.

## Excluded (needs-redesign, per policy - do not port)
- socket module (entire socket type)
- dict view set operations (their dict representation)
- zlib streaming objects (PyCompress/PyDecompress)
- hashlib PyHasher object model (algorithm crates only)
- select poll/epoll/kqueue OBJECTS (PyRef + allow_threads plumbing)
