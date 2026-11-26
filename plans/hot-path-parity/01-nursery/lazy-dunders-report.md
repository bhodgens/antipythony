# Leaf hp-lazydunders — report

Worktree: /Users/caimlas/.git/worktrees/leaf-hp-lazydunders (branch
tier1-hp-lazydunders). Commits: 043e639 (baseline probes) → 06dd383
(step 1) → 96a43b2 (step 2) → e35394c (this report). Baseline numbers +
full after-table: lazy-dunders-baseline.md (same dir).

## What landed

**Step 1 — unregistered attrs payload (ap-obj/object.rs, ap-run/classes.rs).**
Every `PyObject::new` allocated a REGISTERED attrs `Gc<PyDict>`; the
registry insert + GC-pass share was ~470ns of the measured 1413ns mint
floor. The payload is now born unregistered (`PyDict::new_unregistered`),
the same fence posture `dunders` has had since Fix-C: `edges_of` walks
attrs children under the parent PyObject's lock, so the payload's own
(nonexistent) registry entry is never consulted. Fence delta documented:
a cycle through a vars()-handle only (instance dropped, `__dict__`
retained) is no longer collectible — attrs dicts no longer participate as
registry roots. gc_differential + ast_getattr_identity_fence + full gc
corpus green (gc018 fails identically on the base tree — pre-existing
qualname divergence).

**Step 2 — bound-method mint fast path (ap-run/interp.rs `attr_get`).**
A dispatch arm at the top of `attr_get` for Dict/List/Set/Frozen
receivers mirrors the deep walk's answer order exactly: `native_attr`
first (the pydict.update-style markers the deep walk serves through
builtins_modules), then `__doc__`, the family-gated fresh BoundMethod
mint, set/frozenset dunder markers, dict EnumDict `_`-keys — and falls
through to the original walk for everything unanswered. Str receivers
excluded (their surface rides pystr markers + name-gated arms this path
does not replicate).

### Byte-exactness lessons (both caught by gates, both fixed in-tree)

1. First attempt returned `Ok(None)` for unanswered fast-path reads
   instead of falling through → `str.split` died with 'NoneType' object
   is not callable at the call site (deep walk serves split through arms
   the fast path doesn't replicate).
2. Second attempt minted bare `update` BoundMethods for names
   `native_attr` claims → func-less method, same crash on `d.update`
   (caught by fix_json's vendored-enum worker path).
3. CPython identity pins verified and preserved: `d.get is d.get` stays
   False per read (fresh mint, never memoized — memoizing would flip the
   observable).

## Before/after (host M2 Ultra, release, best-of-N)

| probe | before | after |
|---|---|---|
| bm_dict (d.get per read) | ~3488ns | ~2400ns |
| bm_list (l.append per read) | ~1885ns | ~1355ns |
| bm_class (c.m per read) | ~1832ns | ~843ns |
| repeat instance-attr read | ~559ns | ~558ns (flat) |
| raw PyObject mint (C_bound) | ~1423ns | ~841ns |
| dict.get 32k bench row | 14.5ms | 14.3ms |

## Gates (final tree 96a43b2, all re-run first-hand)

- 7 diff gates: fix_int / fix_json / fix_sort / fix_random / diff_b64
  (2132) / diff_hex (1976) GREEN; fix_sha_zlib 27 divergences = the
  documented zlib-rs CORRUPT-message baseline (identical count before).
- ap-conform values 50/50; stdlib 48/48.
- cargo test -p ap-run -p ap-obj -p ap-gc --release: 315 passed /
  0 failed across 27 suites (incl. gc_differential +
  ast_getattr_identity_fence + race-stress).

## Targets vs. delivered (honest)

The leaf's ≤300ns mint / ≤100ns repeat-read targets are NOT met. After
killing the registered-attrs waste (-570ns) and the walk depth
(-1090ns), the remaining ~840ns raw mint floor is Gc alloc + registration
(~450ns, ap-gc batching/nursery territory — the sibling nursery leaf) +
15 interior Mutex/RwLock inits + receiver Arc clone; the ~550ns repeat
instance-attr read is the classes.rs MRO walk (type_ref upgrade +
lookup_with_owner + instance-dict probe) — a type_version-keyed
method/attr cache is the filed lever (02.05b counters already exist).
Both are structural surfaces beyond this leaf's two-attempt budget.
CPython `a.m == a.m` value-eq True vs our identity-eq False is
PRE-EXISTING (verified on base tree).

## Worktree note

Sibling leaf edits (fix_bytes `__getattribute__` arms, bytes.find
int-needle family) landed uncommitted in this worktree mid-session
(05:26) — NOT this leaf's work, left uncommitted, excluded from these
commits via explicit pathspecs. FUZZ_SEEDS.txt test-run side-effect
reverted per the fix-sha-zlib-report discipline. gc018's failure
reproduced on the base tree before dismissing it.
