# NEXT-0210D — session record: scheduler 2b/2c impl design (docs leaf)

Branch tier1-0210d @ main 5e8ccac. Worktree ~/.git/worktrees/leaf-0210design.
Docs-only; no code touched. Gate: orchestrator review.

## What landed

`docs/02-10-scheduler-impl-2b2c.md` — implementation-ready design for Plan
02.10 stages 2b (explicit task stacks + statement poll) and 2c (park/unpark +
10k-task gate), expanding v1 §4 / v2's fiber-era M1 recommendation to
field-level Rust. Contents vs the dispatch asks:

1. **2a compat**: NEXT-0210A.md does NOT exist on 5e8ccac — no scheduler 2a
   landed. Designed against the current shape and recorded the landed
   prerequisite that substitutes for it: PEP-342 fiber generators
   (generators.rs, corosensei + PooledStack, 8 MiB / ~330 KB committed),
   no-GIL substrate. §1 states the 2a contract 2b codes against.
2. **Structs**: Task/TaskSpec/TaskState/RunContext/RunQueue/Permit +
   SchedulerIn/Out protocol (§2), compatible with GenFiber
   `Coroutine<GenInput, GenOutput, (), PooledStack>` (generators.rs:54).
   Started tasks pinned to home P (!Send, v2 M1); scheduler tables hold only
   Send data; !Send fibers live in worker-confined MOUNTED tables.
3. **Poll insertion points** (§3, line-cited @ 5e8ccac): primary P1 at
   interp.rs:1091 fused ahead of touch_line; SAFEPPOINT AtomicU64 word
   shared with the future MMTk STW handshake; quantum from SWITCH_INTERVAL
   (threading.rs:278/:1797); P5 budget re-arm at resume_live; P3 back-edge
   poll kept as fallback. touch_line itself unmodified in 2b (three global
   Mutexes at :1056/:1067/:1074 — see T121 below).
4. **Park/unpark transplant** (§4): ParkKey union over all seven prim kinds
   + Join/TimerCancel/DaemonDrain; per-site table (who wakes whom, line-cited
   threading_call.rs); lost-wakeup protocol = register-under-prim-Mutex +
   park_seq epoch + deferred home-P resume; per-prim corners (cond
   release-save ordering, notify(k) exact count, barrier generation, queue
   pairing); compensation lane for native-blocking (APX_COMP_CAP).
5. **LOCALS re-key** (§5): OS tid → logical ident (`current_logical_ident`);
   RLockOwner::Owned(os_tid) → Owned(Ident) — the four compare sites in
   threading_call.rs (:906/:935/:955/:1064) are the silent-failure audit
   list; native_id = home-P tid at first run, documented; LOCALS cutover is
   one-release (never mixed keys in a run).
6. **10k gate arithmetic** (§6): Task+spec+heap ≈ 224 B/task → 2.2 MB per
   10k unstarted; dominant term = started fibers (8 MiB → 78 GiB VA /
   ~3.2 GiB RSS @ 10k vs today's 625 GiB + 10k kernel threads); gate =
   RSS+VA ratio vs serial reference OR absolute ≤ 4 GiB RSS / ≤ 64 GiB VA;
   APX_MAX_STARTED knob as admission backstop.
7. **T121 interaction** (§7): the bisect's per-statement three-Mutex
   serialization is workload-proportional and UNCHANGED by M:N; the poll
   adds one Relaxed load (noise); the scheduler is the natural home of the
   fix — per-task stmt_ordinal/lineno refactor lands as 2b.3 BEFORE the
   poll can pay off in release profile; needs its own traceback gate.
8. **Landing plan** (§8): 2b.1-2b.4 + 2c.1-2c.5 commit sequence with a gate
   per step; 2c exit = the 02.10 leaf gate proper. Risks §9: carried
   (determinism, nested-suspension tripwire, fiber/GC scan, pinning, flux)
   + new N1-N4 (RLock re-key silent-failure class, ordinal-gap invariant,
   quantum fairness, WORKER_RECLAIM main-thread assumption).

## Defers (explicitly out of scope here)

- MMTk STW wiring (consumes §3.1 bit 0; the gc leaf = v2 02.10c owns it).
- Fiber stack tiering / 02.10e; asyncio park keys (T4; gate half vacuous
  per S-05-19 — annotation carried in §8).
- Stage 2a itself (pool substrate) — own leaf; this doc states its contract.
- Line-anchor re-verification at impl time: the impl leaf re-anchors P1/P2
  on its own HEAD (semantic anchors given; integer lines will drift).

## Honest notes

-Designed against current code because 2a is absent; if 2a lands with
 different Task shapes, §1/§2 must be reconciled before 2b dispatch.
- The §6 arithmetic is analytic (field-by-field estimate), not measured —
  the 2c.5 gate is what makes it binding.
- T121-BISECT's architectural ceiling finding is taken as given (release
  ~1.02-1.06x workload-invariant); no measurement was re-run in this
  docs-only session.
