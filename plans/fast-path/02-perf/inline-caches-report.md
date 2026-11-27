# fp-inlinecache — inline caches for LoadGlobal/LoadAttr/LoadAttrCall

Leaf: plans/fast-path/02-perf/01-inline-caches.md
Branch: tier1-fp-inlinecache2 (worktree ~/.git/worktrees/leaf-fp-inlinecache)
Base: main @ 02617fc
Commits: 62118d8 (impl) + 83f63bd (tests/probe)

## What landed

Per-(thread, CodeObject) inline-cache tables consulted/filled at three
dispatch arms of the bytecode interpreter:

| site | entry stamps | warm hit cost |
|---|---|---|
| LoadGlobal | globals dict epoch + builtins dict epoch + both dict identities, resolved Value | 2 epoch loads + 2 ptr compares + Value clone |
| LoadAttr | class payload ptr + type_version + chain_surgery_version + names-dict epoch, plan + payload | 4 stamp compares + plan answer |
| LoadAttrCall | (receiver tag, name, argc) -> dispatcher decision | 1 magic+argc check (tag re-derived from the live receiver) |

New/changed files:
- `crates/ap-bytecode/src/caches.rs` (new): plan constants (AttrPlan:
  Slow/ClassValue/InstDict/BindFunc; CallPlan: Slow/PrimValues/CallMethod)
  + the invalidation-audit stamp map in one place.
- `crates/ap-bytecode/src/sites.rs`: dense `by_instr` pc->site map — the
  O(1) site finder (a per-hit scan of the site vec would cost more than
  the cache saves).
- `crates/ap-bytecode/src/instr.rs` + lower.rs + decode.rs: LoadAttrCall
  now carries a site id (its lowerer allocs a Call-kind site; the
  comprehension append/add emits too).
- `crates/ap-run/src/ic.rs` (new): typed entries (GlobalIc/AttrIc/CallIc)
  + the RefCell-arrayed IcTable + the stamp collectors
  (class_stamps = ptr + type_version + chain_surgery_version + names
  epoch).
- `crates/ap-run/src/bytecode_interp.rs`: consult/fill arms for the three
  opcodes; hit/fill counters in [BC-STATS] under APX_TIER_STATS=1.
- `crates/ap-run/src/classes.rs`: interp-free classifier shims
  (`is_data_descriptor_scratch`, `descriptor_kind_pub`) — the fill
  classifier must re-derive the slow path's resolution ORDER without an
  `&Interpreter` handle. Same shape as the originals; any change there
  must land in BOTH bodies (documented on the shims).

## No-GIL shape (spec item 4)

Per-thread tables (thread_local keyed by the code-ptr; the compiled-body
Arc pin prevents key aliasing). Warm hits take NO lock and perform ZERO
shared writes; worker threads never share entries. Soundness does NOT
ride the storage: every hit revalidates its stamps against the LIVE
02.05b counters (Relaxed loads, the entry-23 epoch contract) — a stale
entry is a miss, and a miss runs the exact slow path. The alternative
(sharded shared lock-free tables with CAS-stamped entries) was evaluated
on the design level, not measured: it buys memory amortization across
threads but adds a shared cacheline touch per warm hit and a shared
RMW per fill; at the leaf's 4-thread probe scale the per-thread shape
has strictly less cross-thread traffic (none). Contention profile
documented here rather than measured twice (honest gap, below).

Fills use try_borrow_mut — a fill can only collide when a NESTED frame
of the same body is mid-dispatch on this thread; that fill skips (the
site refills on a later non-nested miss).

## The GC rule learned (would have shipped a use-after-free)

The first implementation stored cached Values as raw bit-pairs
(transmute_copy of the 16-byte Value). That is INVISIBLE to the tracing
GC: bits in a heap Vec are not roots, so the collector freed a packed
referent mid-run and the next warm hit reconstructed a dangling handle
(seg repro: hot loop reading `c.x`, EXC_BAD_ACCESS inside
PyDict::get). Fix + rule: cached Values are stored DIRECTLY (Arc
handles keep referents reachable through the conservative floor), never
as bits. This rule is now documented in ic.rs header + commit message.

## Invalidation surface (what each stamp rules out)

- Globals positives: any write into the frame's globals or the builtins
  dict rebinds/removes the name -> dict epoch bumps (PyDict::set/pop
  single bump site). Dict-pair identity catches module swaps.
  The T1-d module-globals / exc-class fallback answers are NEVER
  recorded (they read surfaces the stamps don't guard): the fill arm
  probes `globals.contains_str(name) || builtins.contains_str(name)`
  and records only provenance-verified answers. NameErrors stay fully
  dynamic (negative caching rejected — see ic.rs GlobalIc doc).
- LoadAttr: C.x = / del C.x / method overwrite -> TypeObject::
  bump_class_versions (audit rows 10/11/16 — the 02.05b audited sites in
  classes.rs: attr_set_protocol UserClass arm 4499, interned-Type arm
  4523, class-attr write 4642, delattr 4846; NO missing bump site found,
  classes.rs needed no fix). Subclass chains ride chain_surgery_version
  (MRO max). Names-dict epoch additionally catches direct names writes
  that change HOW a name answers. Instance reads never cache the VALUE
  (the InstDict plan re-probes the live attrs dict per hit — an `o.x =
  new` rebind needs no counter). BindFunc mints fresh per hit (CPython
  identity pin) with the defining class re-derived from the LIVE MRO and
  identity-checked against the entry's owner_ptr.
- LoadAttrCall: the (tag, name, argc) key is a pure function of the
  instruction + the live stack receiver (value-family tags are
  constants) — no mutation surface, no stamp needed. The entry memoizes
  only WHICH dispatcher answers; the dispatcher call itself is identical
  on hit and miss. The memo only engages on shapes NOT already covered
  by the fp-calls sibling's fused fast arms (list.append/dict.get/
  set.add answer before the cache is consulted).

## Measurement (isolation probe: tests/bench/primitive/probes/ic_base.py,
## hp_mint.py pattern; best-of-5, 50k iters, M-series host)

| row | before 02617fc | after | delta |
|---|---|---|---|
| global_read (C) | 114.7ns | 59.4-61.1ns | ~1.9x |
| attr_inst (c.x) | 335.5ns | 128-137ns | ~2.5x |
| method_call_dict (d.get("a")) | 194.7ns | 137-141ns | ~1.4x |
| method_bound (c.m mint) | 856ns | 841-889ns | flat (mint floor) |
| method_call_inst (c.m()) | 1304ns | 1190-1200ns | ~1.1x |
| mint_dict (d.get bare read) | 3694ns | 3218-3906ns | flat (mint floor) |

vs CPython 3.14 same probe: global 19.1ns, attr_inst 21.9ns,
method_call_dict 29.3ns.

HONEST: the leaf's absolute targets (global <=30ns, attr <=60ns) are NOT
met. The remaining cost is not the cache (the hit path is ~5 compares +
a clone): it is the bytecode dispatch loop floor (line-table load, stack
push, for-loop overhead ~40ns/iter in this runtime's tier-0 territory —
the tier1 frame/dispatch work). The IC removed the RESOLUTION cost; the
row costs are now loop-dominated. CPython parity on these rows needs the
loop floor, out of this leaf's scope. The dict.get + attribute-loop rows
are measurably faster (1.4x / 2.5x) — the spec's measurable-improvement
bar is met.

Hit-rate evidence (APX_TIER_STATS=1): ic_base.py runs report
ghit=1399984 gfill=72 ahit=349999 afill=1 chit=0 — i.e. ~1 fill per site
per thread then pure hits, exactly the intended shape (chit=0 because
the probe's fused-call shapes all ride the already-fast fused arms).

## Gates (all re-run on the committed tree)

- fix_bytecode.py: 12/12 three-way GREEN (fix_bytecode exercises
  tree-walk vs bytecode vs CPython).
- fix_int / fix_json (127/0) / fix_sort (42/0) / fix_random: GREEN.
- diff_b64: 2132/0. diff_hex: 1976 cases / 3 mismatches — the 3
  (E.unhex.odd, F.unhex.2args, F.unhex.kw) are byte-IDENTICAL to the
  pre-leaf trunk baseline (verified against target-pptrunk3 binary):
  pre-existing unhexlify error-text divergences, NOT this leaf.
- ap-conform values 50/50 + stdlib 48/48, BOTH tier 0 and tier bytecode.
- cargo test -p ap-bytecode: 52/52. cargo test -p ap-run --release:
  19 suites, 0 failed (including the new ic_invalidation suite).
  (En-route: fix_pass_differential initially failed 7 tests — missing
  debug/ap-run-eval binary under the fresh CARGO_TARGET_DIR; building
  ap-run-eval for the new target dir fixed it, pre-existing harness
  path assumption, not this leaf.)
- Toggle-off: `--tier 0` runs print no IC stats lines and never enter
  the bytecode path (unchanged behavior).
- invalidation gates (NEW, ic_invalidation.rs — 7 tests x BOTH modes):
  set/del/re-add instance attr after warm; set/del/re-add class attr
  after warm; method add + overwrite after warm; base override then
  subclass override after warm (the chain case); global rebind /
  builtins shadow / unshadow / del after warm; fused-call shapes after
  receiver mutation. Each asserts the EXACT post-mutation answer — a
  missed bump shows as a stale value (fail-closed).
- no-GIL probe: 4 threads x 200k reads on shared sites while a mutator
  churns values + performs class surgery concurrently; stale-FREEZE
  detection (IC answer frozen while the walk advances) — green, 3x
  stable.

## Honest gaps

1. Absolute ns targets (30/60) not met — dispatch-loop floor dominates
   the rows now (see measurement table). Filed as tier-1 dispatch work.
2. Contention probe verifies CORRECTNESS under concurrency; the two
   no-GIL shapes (per-thread vs sharded shared) were not both measured —
   the per-thread shape won on design (zero shared traffic) and the
   probe documents its profile. A shared-table measurement would need a
   second implementation.
3. mint_dict/method_bound rows unchanged (~3.3-3.9us mint floor): the
   mint itself (PyObject alloc + registration) is the primitive-perf
   campaign's known BoundMethod-mint row (~1.5us->3.9us here reflects
   the probe's loop overhead) — pre-existing filed lever, untouched.
4. LoadGlobal negatives (NameError paths) uncached by design; a
   cold-tolerant negative cache would need per-answer provenance walks.
5. GlobalLoad sites record only positive answers sourced from the
   (globals, builtins) pair; reads whose provenance is the T1-d
   module-globals fallback stay dynamic (soundness-first subset).
