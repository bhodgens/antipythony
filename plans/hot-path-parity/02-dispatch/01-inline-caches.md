# 01-inline-caches — per-site method/attr resolution cache

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-hp-inlinecache (branch
tier1-hp-inlinecache, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-hp-inlinecache everywhere.

Parent: plans/hot-path-parity/master.md. DEPENDS: 01-nursery leaves
MERGED (cache design reads the post-nursery mint shape). ~90K context.

## Problem

Attribute/method resolution walks the MRO via special_lookup on EVERY
access (4x MRO walk per compare in sort; per attribute read in
loops). CPython 3.11+ and PyPy solve this with inline caches keyed by
receiver type + site. The runtime already has the invalidation
infrastructure: type_version + surgery_version AtomicU64 counters
(02.05b) bump on class mutation.

## Design

1. **Site table**: a process-global site registry (Vec, indexed by
   monotonically-assigned site id stamped at each resolution call
   site in interp.rs/methods.rs via a `cache_site!` const). Each
   entry: (receiver_type_id, resolved_target, type_version_at_fill).
2. **Fast path**: on resolution, probe the site entry; if type id
   matches AND the type's version counters are unchanged -> use the
   cached target directly (one pointer chase + two integer compares).
   Miss -> existing full lookup, then fill.
3. **Invalidation**: type mutation sites already bump versions (02.05b
   audited exhaustively — reuse the same list). No TTL needed.
4. **Scope**: attribute reads on instances/classes, method resolution,
   and the sort comparator's py_lt special_lookup (this directly
   removes the 4x-walk term pp2-sort cannot).
5. Guard rails: single-global-table contention under threads (the
   runtime is no-GIL) — shard by site id or use per-site
   RwLock<entry>; measure both.

## Files you own

crates/ap-run/src/methods.rs (resolution sites), interp.rs (attribute
eval sites), classes.rs (special_lookup), a NEW
crates/ap-run/src/inline_cache.rs. Check git status for sibling holds.

## Tasks

T1 — baseline: ns per attribute read (cold/warm), per method call,
per sort compare on the existing probes. T2 — site table + fast path
+ invalidation on the version counters. T3 — wire the sort comparator
first (direct P08 win), then attribute reads, then method resolution.
T4 — gates (all 7 diff + conform + cargo + GC differentials) and the
thread-race probe (the runtime is no-GIL: hammer one site from 4
threads). Targets: warm attribute read <= 60ns; sort compare overhead
halved; dict.get row improved measurably. T5 — report
plans/hot-path-parity/02-dispatch/inline-caches-report.md.

## Self-verification checklist
- [ ] baseline cold/warm numbers recorded
- [ ] invalidation proven: mutate a class after warm, cache misses
      correctly (unit test)
- [ ] no-GIL thread probe green (4 threads x one site)
- [ ] all gates green; sort compare overhead halved or better
