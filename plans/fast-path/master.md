# FAST-PATH — master plan

**Goal:** Kill the tier-0 walk floor (~190ns/step vs CPython ~13ns) so the
dict/sort/json.loads/join/split rows reach or beat CPython. Method: lower
the AST to a flat bytecode with a computed-goto-style dispatch loop,
thread-local line bookkeeping, inline caches for name resolution, and
superinstructions — all byte-exact, all reusing the existing tree-walker's
helper functions as the single source of semantics.

**Contract (inherited from primitive-perf + hot-path-parity):**
- Byte-exactness is THE gate: every leaf re-runs the 7 diff gates
  (fix_int, fix_json, fix_sort, fix_random, diff_b64, diff_hex,
  fix_sha_zlib) + ap-conform values 50/50 + stdlib 48/48 + cargo suites
  + GC differentials. A bytecode path that diverges by one byte is a
  revert.
- Semantics live in shared helpers (ap-obj ops, methods.rs, interp
  helpers). Both the tree-walker AND the bytecode interpreter call the
  same functions. No semantic duplication.
- The tree-walker STAYS. It is the fallback for shapes the lowerer
  doesn't handle (compute-heavy odd corners, exec/eval edges) and the
  reference implementation for differential testing.
- Machine: Mac M2 Ultra; ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
  worktrees under ~/.git/worktrees/; CARGO_TARGET_DIR per leaf.

## Architecture (the shape of the thing)

```
ruff AST -> ap-ir (Expr tree)  [EXISTS, unchanged]
         -> ap-bytecode (flat Vec<Instr>, const pool, site table)
         -> BytecodeInterp (dispatch loop over Instr, no recursion)
```

- `Instr`: compact enum (u8 discriminant + payload union or two-wide),
  ~40-60 opcodes: LoadFast/LoadGlobal/LoadConst/StoreFast/BinOpInt/
  BinOpStr/Call/GetIter/ForNext/Jump/JumpIfFalse/PopJumpIfTrue... plus
  superinstructions (Load2BinOp, LoadConstCall1...).
- **Line table**: (instr-index -> line) sparse vec, consulted ONLY on
  exception/warning paths — replaces per-node `last_line.set()`.
- **Site table**: one entry per Load/LoadGlobal/LoadAttr/Call site for
  inline caches (keyed by type_version, infra from 02.05b).
- **Frame**: locals as a fixed Vec<Value> indexed by compile-time slot
  (kills the per-read dict walk for locals).
- **Toggle**: `--tier bytecode` / APX_TIER=bytecode env; adaptive mode
  may later choose bytecode over tree-walk per function. Default OFF
  until parity is proven, then flipped.

## Tree

```
plans/fast-path/
  master.md                              # this file
  00-infra/
    orchestrator.md
    01-bytecode-crate.md                 # Instr enum, lowerer, const pool,
                                         #   line table, site table, decoder
    02-frame-and-entry.md                # BytecodeInterp skeleton: dispatch
                                         #   loop, frame, entry toggle;
                                         #   arithmetic+names only, NO calls
  01-core/
    orchestrator.md
    01-control-flow.md                   # Jump/ForNext/BoolOp/Break/Continue
    02-calls-and-attrs.md                # Call/LoadAttr/Subscript via shared
                                         #   helpers + inline caches
    03-exceptions.md                     # Try/except/finally/raise mapping
  02-perf/
    orchestrator.md
    01-inline-caches.md                  # name/attr/call site caches
    02-superinstructions.md              # fused opcodes, peephole on lowerer
  03-rollout/
    orchestrator.md
    01-adaptive-toggle.md                # flip default, perf-vs-treewalk A/B
    02-bench-parity.md                   # full primitive bench + corpus run,
                                         #   report + README figures
```

## Dispatch order (dependencies)

1. `00-infra` — both leaves; 02 depends on 01 (lowerer must exist).
2. `01-core` — 01 (control flow) first; 02 (calls) after; 03 (exceptions)
   last (hardest: needs the Flow::Raise mapping preserved byte-exactly,
   including traceback line attribution via the line table).
3. `02-perf` — after 01-core: caches need Call/Attr sites to exist;
   superinstructions need real opcode-frequency data from a profile run.
4. `03-rollout` — flip default only when every diff gate + full corpus
   + primitive bench pass in bytecode mode.
- Parallel-with-everything: `04-micro` items from hot-path-parity
  (hp-micro BE to_bytes, join/split) stay dispatched opportunistically.

## Byte-exactness strategy (per leaf)

- **Differential both-walkers test**: for every leaf, run the N existing
  diff gates AND a new `fix_bytecode.py` that runs each gate's Python
  probe under (a) tree-walker, (b) bytecode mode, (c) CPython — all
  three must agree byte-for-byte. This catches bytecode-vs-treewalk
  divergence, not just antipythony-vs-CPython.
- **Traceback line attribution**: the line table must reproduce CPython's
  per-instrument line numbers exactly (the s016/s016b/s016c probe family
  pins this — run them in every leaf).
- **Coverage ladder**: arithmetic + names -> control flow -> calls ->
  exceptions -> comprehensions -> generators (generators may stay on the
  tree-walker initially; they are already fast post-02.07).

## Explicit out-of-scope (stays on tree-walker)

- Generators/async frames (fiber machinery — already at parity speed).
- exec/eval with dynamic globals.
- Anything the lowerer rejects: loud fallback to tree-walker, never a
  silent miscompile. Lowerer rejection is a WARN (APX_BP_DEBUG=1), not
  an error.

## Completion Tracking Table

| group | status |
|---|---|
| 00-infra | PENDING |
| 01-core | PENDING |
| 02-perf | PENDING |
| 03-rollout | PENDING |

## Exit gate

Primitive bench in BYTECODE mode: no row below 1.0x CPython among
dict.getitem/build, list.sort, str.join/split, json.loads (the
walk-bound rows); simple-mean ratio >= 1.5x. Full corpus (values
50/50, stdlib 48/48) + all diff gates green in bytecode mode. Report
+ README figures updated.

## Test Gate

Per leaf: the both-walkers differential (above) + the standard battery.
Perf claims only after gates green.

## Coding Conventions

- Instr enum: exhaustive match with `_ => unreachable!()` forbidden —
  every opcode handled explicitly (dispatch-loop safety).
- The lowerer returns Result; unsupported shapes lower to a
  `FallbackToTreeWalk` pseudo-instruction at function granularity.
- No new crate deps. Unsafe only with a SAFETY comment naming the
  invariant (buffer bounds proven at lowering time).
