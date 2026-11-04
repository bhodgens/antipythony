# Leaf A-1: RustPython leaf-donor adaptations, wave 1

## Source
docs/rustpython-leaf-donor-inventory.md (gap table + file:line refs).
Donor clone: /tmp/rustpython-study (MIT). Policy: docs/rustpython-donor-policy.md
(leaf-level only; adapted files keep headers; THIRD_PARTY_NOTICES entry).

## Items (all in crates/ap-run/src/builtins_modules.rs unless noted)
1. errno full table: replace the 56-pair table (~:7480) with RustPython's
   ERROR_CODES (~350 pairs, vm/src/stdlib/errno.rs:40). Data-only.
2. binascii qp/uu/hqx: adapt common/src/binascii.rs (VM-free) into our
   binascii surface; closes quopri + email chains.
3. bytes/bytearray hex/fromhex full semantics: sep/bytes_per_sep args
   (bytes_inner.rs:663-713 - VM-free except 2 error ctors); bytearray.fromhex.
4. stat S_IMODE/S_IFMT callable forms (_stat.rs:274-317 consts).
5. os.pipe/pipe2 + os.kill/killpg: ADD the missing call_os dispatch arms
   (names registered at :6689/:6693 but no arms - fall-through bug class);
   adapt posix.rs:1286/:2296 (libc::kill + signal-name map).
6. os.utime completion: ns=/dir_fd=/follow_symlinks (os.rs:2022 pattern).

## Attribution
- Adapted files: header comment "adapted from RustPython (MIT), <file:line>".
- THIRD_PARTY_NOTICES: xtask notices regen after.

## Gates
- cargo test --workspace 0 failed
- pytest tests/ 1912+
- corpus non-decreasing
- new differential: errno spot-check (5 codes), binascii qp roundtrip,
  bytes.hex sep forms, stat.S_IMODE, os.pipe write/read - add to
  stdlib_modules_differential.rs
