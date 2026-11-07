# IC version audit — mutation-site → version-bump coverage table

Leaf 02.05, Risk 4. A missed version bump under mutation is a CORRECTNESS
bug (stale-wrong caches), not a perf one. This table enumerates every
ap-obj/ap-run mutation site that can invalidate an inline cache and maps
each to its bump. Status column reflects what EXISTS today: the version
counters land with the tier-1 typed-codegen wave; the IC substrate
(`feedback.rs`) and this audit table land FIRST so every counter that
appears has a registered consumer and a bump-coverage owner.

## Version domains (pon abi.rs:530-556 analogue)

| Domain | Counter owner | Guards which IC | Seeded |
|---|---|---|---|
| Type version | per ap-obj type object (`PyObject` type slots) | `AttrIC` (attribute offset/descriptor cache on instances of that type) | 1 |
| Dict version | per `PyDict` (already exists: `PyDict::mutation_epoch`, dict.rs:224) | `GlobalIC` module-globals caches, instance `__dict__` shape caches | 1 |
| Class-surgery version | per class (bases, MRO, `__dict__` swap) | `AttrIC` descriptor chains, method caches | 1 |

`PyDict::mutation_epoch` is ALREADY the dict-version counter: every
store/del/clear/update path in dict.rs bumps it (verified at HEAD:
`set` bumps, `pop` bumps, `clear` bumps, `update` bumps via `set`).
This is the counter the GlobalIC reads; no new dict counter is added.

## Mutation-site → bump table (the audit surface)

| # | Mutation site | File (owner) | Must bump | Status |
|---|---|---|---|---|
| 1 | `obj.attr = v` (instance dict store) | ap-run/classes.rs setattr route → `PyDict::set` | dict version (`mutation_epoch`) | COVERED — `PyDict::set` bumps |
| 2 | `del obj.attr` | ap-run/classes.rs delattr → `PyDict::pop` | dict version | COVERED — `pop` bumps |
| 3 | `d[k] = v` dict store | ap_obj/dict.rs `PyDict::set` | dict version | COVERED |
| 4 | `del d[k]` | ap_obj/dict.rs `PyDict::pop` | dict version | COVERED |
| 5 | `d.clear()` | ap_obj/dict.rs `PyDict` clear path | dict version | COVERED (bumps via store-removal path) |
| 6 | `d.update(...)` (incl. self-alias snapshot path, 3a80a4b) | ap_obj/dict.rs `update` → `set` loop | dict version | COVERED — routes through `set` |
| 7 | `dict.popitem()` | ap_obj/dict.rs `popitem` | dict version | COVERED |
| 8 | `dict.setdefault` | ap_obj/dict.rs → `set` | dict version | COVERED |
| 9 | EnumDict backing-dict virtuals (02.03b `__ap_dict_*__` routes: `__ap_dict_get__`/`__ap_dict_set__`/`__ap_dict_del__` on enum internals) | ap-run/classes.rs enum shim → same `PyDict` primitives | dict version | COVERED — virtuals MUST route through `PyDict::set/pop` (audit rule: no raw-bucket mutation outside dict.rs) |
| 10 | class surgery: `C.__dict__ = ...`, base insert/remove, `__mro__` rewrite | ap-run/classes.rs type-mutation paths | class-surgery version (+ type version for subclass chains) | PENDING (counters land with tier-1 wave; sites enumerated here so coverage is checkable) |
| 11 | metaclass hooks mutating instance/class layout (`__set_name__`, `__init_subclass__`, `__slots__` resolution) | ap-run/classes.rs | type version on the affected type | PENDING (same wave) |
| 12 | `__prepare__` namespace replacement | ap-run/classes.rs class-creation pipeline | dict version on the REPLACED namespace | COVERED if replacement stores through `PyDict`; audit rule: `__prepare__` must return a `PyDict`-compatible surface that bumps |
| 13 | `super_stack` attribute caching (02.03b) | ap-run/classes.rs super dispatch | type version on `__class__` cell target | PENDING (same wave) |
| 14 | cell stores (`__classcell__` fill, closure cells) | ap-run/classes.rs + frame.rs | none (cell contents are not IC-guarded; identity-stable) | N/A — documented exception |
| 15 | `gc` finalizer install/remove (`__del__` surgery) | ap-gc weakref/finalizer registry | type version (descriptor-ish cache) | PENDING (same wave) |
| 16 | `setattr` on a CLASS (method add/remove — the hot AttrIC invalidator) | ap-run/classes.rs `type.__setattr__` | type version + class-surgery version | PENDING (same wave) |
| 17 | module global store/delete (`x = ...` at module scope, `del x`) | ap-run frame.rs module-namespace route → `PyDict` | dict version (module `__dict__`) | COVERED via `PyDict::set/pop` once routed through them; audit rule: module stores MUST use `PyDict` |
| 18 | `builtins` shadowing changes | importer.rs bootstrap | builtins_version (GlobalIC second guard) | PENDING (rare path; lands with GlobalIC consumer) |

## Audit rules (binding for future waves)

1. No raw-bucket mutation of `PyDict` outside dict.rs — every store/del
   goes through `set`/`pop`/`clear` so `mutation_epoch` bumps. The
   EnumDict backing-dict virtuals and the 02.03b `__ap_dict_*__` routes
   are REQUIRED to route through these primitives (rows 6, 9).
2. Every new type/class mutation path lands WITH its counter bump + a row
   in this table, or the path is rejected in review.
3. Debug assertion mode `APX_IC_AUDIT=1` (audit_probe.rs): on every IC
   hit, cross-check the cached value against a slow-path recompute;
   mismatch = panic in debug (fail-closed), error-log in release.
4. `PyDict::mutation_epoch` is the single dict-version source of truth.
   Do not add parallel dict counters.

## Probe matrix (Risk-4 adversarial suite — wired with the counters)

mutate-through-every-path after an IC warms; cached value must match the
slow path:
- instance attr set/del × (plain class, slots class, enum member, class
  with metaclass `__setattr__`)
- dict store/del/clear/update/setdefault/popitem × (plain, EnumDict-
  backed, `__prepare__`-replaced namespace)
- class method add/remove after cache warm (rows 10, 16)
- super_stack route after class surgery (row 13)
- gc finalizer add/remove after attr-IC warm (row 15)
