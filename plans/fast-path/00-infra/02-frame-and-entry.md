# 02-frame-and-entry — BytecodeInterp skeleton: dispatch loop, frame, entry toggle

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-frame (branch
tier1-fp-frame, from current main). Own commits, explicit pathspecs.
ONE heavy cargo lane; CARGO_BUILD_JOBS=4; CARGO_TARGET_DIR=$PWD/
target-fp-frame everywhere.

Parent: plans/fast-path/master.md. DEPENDS: 00-infra/01-bytecode-crate
MERGED. ~100K context.

## Deliverable

1. **BytecodeInterp** (in ap-bytecode or ap-run — your call, document
   it): a dispatch loop over Vec<Instr> with:
   - a fixed-size locals Vec<Value> (slot-indexed),
   - an operand stack Vec<Value>,
   - an explicit `pc` (no recursion for straight-line code — nested
     CALL opcodes recurse into call_value like the tree-walker does;
     that's fine, calls were never the walk floor),
   - **line bookkeeping**: on each instruction, write the current line
     to a thread-local Cell<u32> (NOT the lock-guarded frame atomic —
     that's the 23-stores-per-node tax). The shared frame's
     last_line is updated ONLY on: exception construction, warning
     emission, and yield points — read from the thread-local at those
     moments. Traceback/warning byte-exactness must be proven by the
     s016/s016b/s016c probe family.
2. **Semantics via shared helpers**: BinOp -> ap_obj::ops (the SAME
   functions the tree-walker calls). Name resolution: Fast -> locals
   vec; Global -> the same globals dict the tree-walker uses.
3. **Entry toggle**: `--tier bytecode` CLI flag + APX_TIER=bytecode
   env. When a function's bytecode is available and the toggle is on,
   the function executes via BytecodeInterp; otherwise falls back to
   the tree-walker. Default behavior unchanged when toggle is off.
4. **Execution coverage**: arithmetic, names, constants, comparisons,
   attribute/subscript reads via the existing interp helpers. Calls
   may route through call_value (correct but slow — calls are leaf
   01-core/02's job to optimize). Control flow opcodes present in the
   enum but functions containing For/While/Try still fall back
   (01-core's job).
5. **Both-walkers differential**: new gate `fix_bytecode.py` — runs a
   probe file under (a) tree-walker, (b) bytecode mode, (c) CPython;
   all three outputs must be byte-identical. Probe set: arithmetic
   (int/float/mixed, div/mod sign rules), string ops, comparisons,
   locals vs globals scoping, shadowing, closures reading enclosing
   locals, line-number-sensitive tracebacks (raise inside nested
   calls; assert the traceback line numbers match CPython exactly).

## Gates (both-walkers + standard)

- fix_bytecode.py GREEN (three-way identical).
- All 7 existing diff gates GREEN with the toggle ON and OFF.
- ap-conform values 50/50 + stdlib 48/48 with toggle ON (this is the
  big one — functions that lower successfully now execute on the new
  path; any byte divergence is a bug in the dispatch loop).
- s016/s016b/s016c line-attribution probes GREEN (line table proof).
- cargo test -p ap-bytecode -p ap-run green.

## Files you own

crates/ap-bytecode/src/interp.rs (new), ap-run wiring (jit_hook.rs or
a new entry module), CLI --tier flag extension, tests.

## Self-verification checklist
- [ ] dispatch loop: no recursion for straight-line code; explicit pc
- [ ] line bookkeeping: thread-local Cell on hot path; frame atomic
      only at exception/warning/yield
- [ ] s016 family green (traceback line numbers byte-exact)
- [ ] fix_bytecode.py three-way identical
- [ ] all 7 diff gates + conform green with toggle ON
- [ ] toggle OFF = bit-identical behavior to main (zero-risk default)
