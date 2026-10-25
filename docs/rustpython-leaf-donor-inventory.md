# RustPython leaf-donor inventory (gap scan, 2026-10-24)

Source: shallow clone at `/tmp/rustpython-study` (RustPython @ main, MIT).
Donor policy: docs/rustpython-donor-policy.md — leaf-level, architecture-neutral
code only. Parser/AST/eval loop/object model/GC/import machinery are NOT donors.
Anything typed on their `VirtualMachine`/`PyObject` is marked needs-redesign
below even when the algorithm is portable.

## Gap table

MODULE | RustPython has (file:line) | We have
--- | --- | ---
bytes.hex | vm/src/builtins/bytes.rs:377, vm/src/bytes_inner.rs:663 (`bytes_to_hex`, sep/bytes_per_sep) | partial — `hex` dispatched (builtins_modules.rs:11592), no sep/bytes_per_sep args
bytes/bytearray.fromhex | vm/src/bytes_inner.rs:667 (pure `fromhex(&[u8])`), :716 (`fromhex_object`, str-or-buffer) | Y — classmethod dispatch (interp.rs:8578); whitespace/position-error semantics from RP worth diff-checking
bytearray methods | vm/src/builtins/bytearray.rs (full set, shares bytes_inner) | partial — bytearray shares the `pybytes` surface; no resize/mutable-buffer semantics
os.utime | vm/src/stdlib/os.rs:1955 + utime_impl:2022 (ns=, dir_fd=, follow_symlinks=) | partial — builtins_modules.rs:6889 (set_modified only; atime ignored, no ns=/dir_fd=)
os.pipe / pipe2 | vm/src/stdlib/posix.rs:1286 / :1301 (returns OwnedFd pair) | N — name registered (builtins_modules.rs:6689) but call_os has no `pipe` arm
os.kill / killpg | vm/src/stdlib/posix.rs:2296 / :2301 (libc::kill wrapper + signal-name mapping) | N — name registered (:6693), no call arm
os.stat / fstat / lstat | vm/src/stdlib/os.rs:1585 / :1601 / :1611 (dir_fd, follow_symlinks) | partial — str-only stat/lstat; no dir_fd/follow_symlinks
os.dup / dup2 | vm/src/stdlib/posix.rs:2326 / :2341 | N — not on surface
os.uname | vm/src/stdlib/posix.rs:1772 (structseq fields) | partial — platform.py shim path (builtins_modules.rs:7612)
os.path.realpath | pure string walk (posixpath semantics) | partial — builtins_modules.rs:7134 (str-only, no symlink resolution)
base64 urlsafe_b64encode/decode | vm crates/stdlib/src/binascii.rs:243 (alphabet attr) + a2b/b2a_base64 :200/:336 | Y — hand-rolled alphabet map (builtins_modules.rs:8709)
binascii core | crates/common/src/binascii.rs:83-739 — hexlify, unhexlify, crc32, crc_hqx, a2b/b2a_base64, a2b/b2a_qp, a2b/b2a_uu | partial — b64/b16/encodebytes + crc32; no qp/uu/hqx
zlib | crates/stdlib/src/zlib.rs:63-295 (adler32, crc32, compress, decompress, compressobj, decompressobj; flate2-backed) | N — import-level stub only (docs/t1-gate.md known gap)
hashlib | crates/stdlib/src/hashlib.rs (md5/sha1/sha2/sha3/blake2/shake via RustCrypto, hmac :1191, algorithms_available) | partial — md5/sha1/sha2 family only; hexdigest-callable pattern is a known T2 gap; no blake2/sha3/shake/algorithms_*
errno | vm/src/stdlib/errno.rs:40 (`ERROR_CODES`, ~350 name/code pairs + errorcode dict) | partial — 56 pairs + errorcode (builtins_modules.rs:7480)
stat constants/fns | vm/src/stdlib/_stat.rs:274-317 (S_IFMT/S_IMODE/S_IS* as consts) | partial — constants + S_IS* + filemode (builtins_modules.rs:7384); missing S_IMODE/S_IFMT callable forms
socket constants | crates/stdlib/src/socket.rs:83-100 (AF_*/SOCK_*/SOL_*/TCP_* via cfg'd const tables) | partial — constants in vendored_stdlib/socket.py; no native socket module
select poll/epoll/kqueue | crates/stdlib/src/select.rs:283 (poll object), :417 (epoll), :665/:679 (kqueue/kevent) + crates/host_env/src/select.rs (libc shims) | partial — vendored select.py raises OSError for kqueue/epoll; select.select/poll shim per t1-gate
dict view objects | vm/src/builtins/dict.rs (real view types: set ops, rich compare) | partial — marker types dict_keys/values/items (builtins_modules.rs:12549), no set operations
property.setter | vm/src/builtins/descriptor/getset.rs, property.rs | Y — S-03-10 landed (classes.rs:1587)

## Adaptation-cost ratings (leaf-level only)

- trivial (<30 lines, copy-adapt): bytes hex/fromhex core (bytes_inner.rs:663-713
  is VM-free except 2 error constructors); stat S_IMODE/S_IFMT consts; errno
  table extension (data only); kqueue/epoll/poll "unsupported" error types.
- small (30-100 lines): os.kill/killpg + signal-name map (posix.rs:2296, needs
  libc::kill + our OSError shim); os.pipe/pipe2 (posix.rs:1286, OwnedFd -> our
  fd ints); utime ns=/dir_fd= completion (os.rs:2022 pattern); binascii
  qp/uu/hqx from common/src/binascii.rs (769 lines, ZERO VM deps — the single
  cleanest donor file in the tree); errno full table (~350 entries).
- needs-redesign (depends on their VM types): select poll/epoll/kqueue objects
  (PyRef + allow_threads plumbing); hashlib object model (their PyHasher class
  machinery — algorithm crates are portable, object pattern is not); dict view
  set operations (their dict representation); zlib streaming objects (their
  PyCompress/PyDecompress classes; flate2 calls themselves are trivial);
  socket module (entire socket type).

## Top-10 adaptations (ranked: closes known T1/T2 gap x trivial cost)

1. errno full table (errno.rs:40) — data-only, closes subprocess/socket error
   text mismatches. trivial.
2. binascii qp/uu/hqx core (common/src/binascii.rs) — VM-free, closes quopri
   and email chains; we already have the module shell. trivial-to-small.
3. bytes/bytearray hex(fromhex) full semantics incl. sep/bytes_per_sep and
   bytearray.fromhex — closes T1 bytes-surface differential. trivial.
4. os.pipe/pipe2 (posix.rs:1286) — closes subprocess pipe chains; small.
5. os.kill/killpg + signal map (posix.rs:2296) — closes signal module shim
   callers; small.
6. utime ns=/dir_fd=/follow_symlinks completion (os.rs:2022) — closes shutil
   copy2 precision gap; small.
7. hashlib algorithms_available/guaranteed + sha3/blake2 via RustCrypto crates
   (algorithm layer only) — closes T2 jwt/werkzeug hash chains; small per alg.
8. stat S_IMODE/S_IFMT callable + missing UF_/SF_ constants (_stat.rs) — trivial.
9. zlib one-shot compress/decompress via flate2 (zlib.rs:98/:120 bodies) —
   upgrades import stub to real compression without the streaming objects; small.
10. os.dup/dup2/fstat fd family (posix.rs:2326+) — unblocks fdopen/select
    integration tests; small.

## python_tests/ mining list (Lib/test/)

- test_bytes.py (3090 lines) — bytes+bytearray method conformance incl. hex/fromhex edge cases.
- test_base64.py (1451 lines) — b64/b32/b16/urlsafe + strict_mode/canonical.
- test_binascii.py (1642 lines) — hexlify, crc32, qp, uu round-trips.
- test_hashlib.py (1422 lines) — algorithm coverage, usedforsecurity, shake XOF.
- test_os/ (5 files: test_os.py, test_posix.py, utils.py) — utime, pipe, kill, stat dir_fd cases.
- test_stat.py (339 lines) — filemode/S_IS*/S_IMODE conformance table.
- test_errno.py (33 lines) — errorcode dict invariant.
- test_zlib.py (1118 lines) — compress/decompressobj streams, checksums.
- test_select.py (104 lines) + test_selectors.py — select/poll semantics.
- test_posixpath.py, test_genericpath.py — realpath/normpath/canonicalize edge cases.
- test_socket.py (7650 lines) — constants + socket API (mining: constant tables only).

## Notes

- The cleanest donor is `crates/common/src/binascii.rs`: no VM types at all,
  769 lines, MIT. Copy-adapt with attribution header.
- RP's error messages in bytes.fromhex ("non-hexadecimal number found ... at
  position i") match CPython exactly — worth mirroring verbatim.
- Their select shim structure (crates/host_env/src/select.rs) is a good
  architecture sketch for our kqueue-on-macOS shim, but is needs-redesign.
