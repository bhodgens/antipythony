# Leaf hp-lazydunders — T1 baseline (recorded BEFORE changes)

Worktree: /Users/caimlas/.git/worktrees/leaf-hp-lazydunders
Base commit: 28480a0 (branch tier1-hp-lazydunders, from main)
Build: release, CARGO_TARGET_DIR=target-hp-lazydunders, quiet-ish window.

## Bound-method mint + attr read (tests/bench/primitive/probes/hp_mint.py, N=200k, best-of-5)

| probe | antipythony | CPython 3.14.7 |
|---|---|---|
| bm_dict (`d.get` per read) | ~3488 ns | ~52 ns |
| bm_list (`xs.append` per read) | ~1885 ns | ~52 ns |
| bm_class (`c.m` per read, MRO walk) | ~1832 ns | ~45 ns |
| attr_inst (repeat instance-attr read `c.x`) | ~559 ns | ~21 ns |
| attr_miss (AttributeError path) | ~1565 ns | ~265 ns |

## Raw mint decomposition (ap_mint_eval probe bin, N=160k)

| label | per-op ns | note |
|---|---|---|
| A_list (PyList mint alone) | ~430 ns | registry + GC amortized |
| B_pyobject (PyObject::new) | ~1413 ns | attrs Gc<PyDict> (REGISTERED) mint = the delta vs A |
| C_bound (with_self BoundMethod) | ~1423 ns | ≈ B + receiver lock |
| D_dict set+get (interned-key table shape) | ~90 ns | the fast-path target |
| E_dict get-only | ~69 ns | |

Root: every PyObject mint allocates a REGISTERED attrs dict (registry insert
≈ 985ns floor per fix-c-report) that most objects never use. BoundMethod
mints pay it on EVERY primitive-method attribute read.

## Gates BEFORE (fresh build, this tree)

- ap-conform values: 50/50 PASS
- ap-conform stdlib: 48/48 PASS
- 7 diff gates: to re-run after changes (fix_int/fix_json/fix_sort/
  fix_random/fix_sha_zlib/diff_b64/diff_hex)

## CPython semantics pins (3.14.7, probed)

- `d.get is d.get` on the TYPE: True. On an INSTANCE: False (fresh
  BoundMethod per attribute read — `inst.m is inst.m` False, `==` True via
  default object identity? NO: == on methods is value-eq: `a.m == a.m`
  True). Repeat-read caching a bound method per instance changes
  `a.m is a.m` False->True — NOT byte-exact, so the fast path must NOT
  return the same Value for repeat instance-method reads. The cacheable
  fast path is: skip re-walking + skip payload mint cost only if identity
  is preserved (fresh mint per read). Correct structural lever = make the
  FRESH MINT cheap (lazy attrs/dunders), not memoize the Value.
