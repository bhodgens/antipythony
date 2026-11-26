# FAST-PATH — orchestrator (00-infra)

Foundation leaves: the bytecode crate (IR->bytecode translation) and
the dispatch interpreter with the entry toggle. Everything else in the
plan builds on these.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-bytecode-crate.md | new ap-bytecode crate: Instr, lowerer, const pool, line table, site ids | none |
| 02-frame-and-entry.md | BytecodeInterp dispatch loop + frame + --tier bytecode toggle + both-walkers gate | 01 |

## Dispatch protocol

1. 01 first (pure addition — new crate, zero risk to existing gates).
2. 02 after 01 REVIEWED and MERGED. 02 is the first leaf that changes
   execution behavior (behind a toggle, default OFF).
3. The both-walkers differential (fix_bytecode.py, three-way: treewalk
   / bytecode / CPython) is born in 02 and extended by every
   subsequent leaf. It is the plan's core safety net.

## Test Gate

01: cargo test -p ap-bytecode + workspace build green.
02: fix_bytecode.py three-way identical + s016 line-attribution
family green + all 7 diff gates + conform in BOTH toggle states.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-bytecode-crate.md | PENDING |
| 02-frame-and-entry.md | PENDING |
