# 02.09c — unboxed PyList-of-Smi storage (the leaf record)

Base: e664aa9a (02.09b complete: the flip, decision-2 per-object locks,
PyList items+allocated under one Mutex). Authoritative spec:
SPLIT-02.09.md §02.09c + docs/0209-object-state-audit.md §4 (census verdict
GO, Smi-only v1, items()-escape-hatch option iii).

## Design (as landed)

### The storage variant

`PyList.items: Mutex<ListStorage>` where

```
enum ListStorage { Boxed(Vec<Value>), Smi(Vec<i64>) }
```

- **Arm selection**: `from_vec` classifies its input — unboxed iff EVERY
  element is `Value::Int(Int::Smi(_))`. Bools are deliberately NOT unboxed
  (bool ⊂ int for arithmetic but `type()`/repr must keep saying bool);
  bigints (`Int::Big`), floats, strs force the boxed arm. `PyList::new()`
  starts boxed (an empty raw array saves nothing).
- **Write gate / demote (one-way Smi → Boxed)**: `append`/`set_at`/
  `insert_at` push a Smi directly into the raw array when storage is unboxed;
  any other value demotes FIRST, inside the same critical section. Demotion
  rebuilds `Value::Int(Int::Smi(n))` per element — exact round trip at both
  smi boundaries (unit-pinned). Never re-unbox: proving re-unboxability after
  arbitrary writes costs the scan the gate avoids.
- **Reads never demote**: `ItemsRef` materializes a temporary boxed snapshot
  at acquisition, UNDER the same lock guard, when the storage is unboxed.
  This is census §4 option (iii) — the escape hatch lives in the guard, so
  all ~178 `.items()` call sites stayed source-identical (verified by
  compile: zero call-site edits).
- **Lock discipline (02.09b decision-2, inherited)**: the variant adds NO
  lock of its own. items storage + growth bookkeeping + the demote
  transition all synchronize on the SAME `Mutex` the flip installed; a
  concurrent `items()` read can never observe a half-migrated list. T120/
  T122 pass unchanged.
- **`list(range(N))` fast path**: `PyList::from_smi_raw(Vec<i64>)` +
  a `builtin_list` arm for Range objects materialize straight into raw
  storage with NO boxed phase. Length math reuses `crate::iter::range_len`
  (fast path must agree with the generic iterator on every input);
  out-of-smi-window values fall back to the generic path byte-identically.

### GC fence invariant (audit §4 — the subtle one)

unboxed ⇒ the list holds NO `Value` refs ⇒ its internal-element edges
vanish from `count_internal_refs`/`edges_of`. The gated invariant:

**unboxed ⇒ no refs ⇒ STILL REGISTERED for container identity.**

- Registration happens in the constructors (`gc_register_pub`) independent
  of storage arm — gc.is_tracked parity with boxed lists (corpus gc007 pins
  `[1, 2]` tracked True).
- Soundness of the missing edges: a raw i64 has no registry address, so no
  edge COULD have been counted — subtract-refs never under-counts fan-in
  for any other object.
- Collection semantics are arm-independent: an externally-held unboxed list
  survives a pass (sc 2 > internal 0 + 1); a registry-only unboxed list is
  swept (sc 1, not seeded, dies).
- Pinned by `gcmod::unboxed_list_stays_registered_with_zero_internal_edges`
  (zero-edge count + registration + both collection halves).

Aliasing/S-01-19 and repr/S-01-44 are demote-or-handle: `clone()` is deep in
both arms (i64 memcpy on the Smi arm — part of the win); an unboxed list
cannot self-reference (raw i64s hold no handles) but `fmt_repr` keeps the
`active` protocol (the list is still registered; the repr path is shared
with boxed lists that CAN cycle). sort/aliasing self-mutation goes through
the same gated mutators as any other write.

## Files

- crates/ap-obj/src/value.rs — the entire variant (PyList region only; the
  a/b split discipline held: no PyString/Cell-cache or flip edits)
- crates/ap-run/src/builtins.rs — the range fast path in `builtin_list`
- crates/ap-gc/src/gcmod.rs — the fence test (tests module only)
- plans/antipythony/02-native-backend/PROGRESS-0209c.md — this record

## Gate results

- **method_surface_differential 34/34** (the 71 list-test surface — zero
  call-site edits required, byte-exact vs live CPython 3.14)
- **repr_fuzz 7/7, cpython_differential 25/25** (ap-obj)
- **gc slice 20/20** (`tests/corpus/gc/` vs live python3.14, ap-conform)
- **gc_differential 2/2** (incl. ast_getattr_identity_fence)
- **T120/T122 PASS**; threading slice spot-green
- **Full corpus 248/287 — ZERO new failures** vs the 02.09b trunk baseline
  measured side-by-side on the same run (37 fails + 2 timeouts identical,
  byte-for-byte reproduction; the 37 are the known pre-existing classes/
  values/codecs reds recorded in PROGRESS-0209b.md)
- **clippy -p ap-obj: 0 warnings**; ap-run clippy unchanged from trunk on
  the touched regions (one pre-existing unused_mut at interp.rs:10164 and
  unused import at classes.rs:26 — both present at main, verified by stash)
- **cargo check --workspace**: clean
- targeted byte-probes (ours vs live CPython 3.14, byte-exact):
  21-case unboxed-arm list surface + 15-case range fast-path surface
  (negative step, empty, bigint-clamped bounds, slice/append/set_at mixes)

## Memory win (measured, NOT gate-pinned per the split)

Debug builds, `/usr/bin/time -l` max RSS, 2M-element int lists, ours vs the
02.09b trunk binary side-by-side on this M2 Ultra:

| probe | ours | trunk | delta |
|---|---|---|---|
| `list(range(2_000_000))` | 30.4–31.8 MB | 47.7–50.1 MB | **-17 to -19 MB (~-36%)** |
| append-loop 2M ints | 34.5 MB | 50.7 MB | **-16.2 MB (~-32%)** |

Exactly the predicted 16 B/elem → 8 B/elem halving of the element array.
First attempt (borrowing classifier + no range fast path) measured +15 MB —
the ctor path double-materialized (boxed vec alive while the raw array
drained); the consuming classifier + `from_smi_raw` range arm fixed it.
An intermediate state (consuming classifier, no range arm) still peaked at
66 MB — the boxed phase for `list(range(N))` dominates; the fast path is
what realizes the win on this probe shape.

## En-route incident (infra, not code)

`crates/ap-run/src/mbc_tables.rs` (generated, 5 MB) was truncated to 291
bytes twice mid-session by an external process (no process of this leaf was
running at either timestamp; the 02.09b record notes sibling mbc WIP was
stashed — a sibling session's gen-tables run likely targeted this worktree).
Restored from HEAD both times via `git checkout --`; final tree verified
clean (4964261 bytes, unmodified). No leaf code was ever affected.

## Residuals (filed, not leaf-blocking)

- Bool elements never unbox (v1 Smi-only verdict: correct — but a
  `bool-list` variant could halve those too if a corpus driver ever needs it).
- No re-unboxing on clear()+refill (`clear()` resets to Boxed-empty;
  a from_vec-shaped rebuild re-unboxes, an append refill does not).
- `iter::collect` still builds a `Vec<Value>` before `from_vec` for
  non-range iterables (generators etc.) — transient boxed phase remains for
  those; only range gets the stream-classification. A future
  `iter::collect_into_storage` could extend the win.
- bytes/str-ascii variants deferred per the census verdict (Smi-only v1).
