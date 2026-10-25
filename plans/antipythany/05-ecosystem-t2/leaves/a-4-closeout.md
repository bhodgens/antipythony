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
2. binascii b64 family completion: b64decode/b64encode with
   validate=/altchars=, standard_b64decode, a2b_base64/b2a_base64
   (they are the real names; b64decode etc. too). b16encode/b16decode
   with casefold= (3.14 still HAS b16encode/b16decode - only the
   donor's newer-3.x kwargs are absent; b16decode MISSING on impl is a
   real gap, the earlier 'removed in 3.14' note applies to the hqx
   family ONLY). Probe python3.14 for every name before implementing.
3. blake2 keyed/tree params: key/salt/person/fanout/depth/digest_size
   via RustCrypto blake2 keyed API; pin error texts vs python3.14.
4. stat.filemode setuid/setgid/sticky: '?rwxr-xr-x' must be
   '?rwsr-xr-T' shape (0o4755 -> ?rwsr-xr-x, 0o2755 -> ?rwxr-sr-x,
   0o1755 -> ?rwxr-xr-t; sticky without exec lowercase t). Probe 3.14
   for the full table incl. the 'l' case for symlinks.
5. Vendored DEVIATIONS cleanup (mechanical): stdlib-provenance --check
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
