# Leaf A-4: RustPython campaign closeout - bytearray representation + small surfaces

## UNBLOCK CONDITION
Dispatch only after A-3 lands (builtins_modules.rs and the interp
type/attr layer are A-3-owned until then).

## Items
1. bytearray representation in ap-obj (THE remaining big residual).
   Today: `bytearray(b'ab')` and `bytearray.fromhex('ab')` produce a
   Value::Bytes - type(ba).__name__ == 'bytes', repr shows b'ab'.
   Repro pinned 2026-10-25. CPython: type 'bytearray', repr
   bytearray(b'ab'), mutable (ba[0] = 65; += works), bytearray-specific
   methods (append/extend/insert/pop/remove/reverse/clear/resize...),
   decode/hex/fromhex shared surface already partly present.
   Options (implementer decides, document in report):
   (a) ObjKind::Bytearray wrapping a Rc<RefCell<Vec<u8>>> with method
       dispatch through the existing bytes method table plus mutation
       methods;
   (b) a type-tag on Value::Bytes.
   CONSTRAINT: value-equality with bytes (ba == b'ab' is True in
   CPython; dict/set keying hashes equal), and every existing
   Value::Bytes match site must still compile - grep the match arms
   first and count them before choosing.
2. binascii base64 family: CPython 3.14 has a2b_base64/b2a_base64 ONLY -
   probed 2026-10-25: b16decode/b16encode/b64decode are ALL ABSENT from
   python3.14 (the b64/b16 alias names were removed in 3.14; earlier
   mining findings claiming those gaps were probing the aliases). The
   impl lacking them MATCHES CPython - do not add them. Verify our
   a2b_base64/b2a_base64 surface (validate=/altchars= kwargs) against
   python3.14 and complete only what CPython actually exposes.
3. blake2 keyed/tree params: key/salt/person/fanout/depth/digest_size
   via RustCrypto blake2 keyed API; pin error texts vs python3.14.
5. stat.filemode setuid/setgid/sticky: '?rwxr-xr-x' must be
   '?rwsr-xr-T' shape (0o4755 -> ?rwsr-xr-x, 0o2755 -> ?rwxr-sr-x,
   0o1755 -> ?rwxr-xr-t; sticky without exec lowercase t). Probe 3.14
   for the full table incl. the 'l' case for symlinks.
5b. os.uname() / os.stat() SUBSCRIPTING: u[0] raises TypeError
   ('object' object is not subscriptable) while len(u)==5 works and
   os.stat st[0] works - the uname_result dispatch path misses the
   Opaque slice_parts subscript arm that stat_result got in A-3.
   CPython structseq: u[0]=='Darwin', u[4]==machine. Probe both.
- Vendored DEVIATIONS cleanup (mechanical): stdlib-provenance --check
   has 121 baseline problems incl. select.py/zlib.py now shadowed by
   native modules. Script the deviated/stale entry updates per the
   documented procedure; run --fix then hand-write the residual
   [[file]] blocks; gate: stdlib-provenance --check exit 0.

## Gates
- Probes: bytearray repr/type/mutation/equality, b64/b16 roundtrips,
  blake2b(key=...) digest vs python3.14, filemode table, all byte-exact.
- Differential cases appended to stdlib_modules_differential.rs.
- cargo workspace 0 failed; pytest >= current count; corpus
  non-decreasing; notices --check exit 0; stdlib-provenance --check
  exit 0 (item 5).

## Notes
- The campaign's policy-excluded list (socket, dict view set-ops,
  streaming zlib objects, select kqueue object, PyHasher model) stays
  excluded - this leaf does NOT reopen any of it.
- After this leaf: the trivial/small inventory from
  docs/rustpython-leaf-donor-inventory.md is fully ported or explicitly
  excluded; campaign closes.
