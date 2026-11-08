# Plan 03 - WasmGC backend: progress

## Status: DESIGN WAVE LANDED (2026-10-30, uncommitted)

- Design doc: `docs/03-wasm-representation-design.md` — pyobj WasmGC struct
  hierarchy, bigint strategy (struct-based in-module, decision recorded),
  exception mapping (Wasm EH exnref, wasmtime >= 47 pin; >= 37 behind
  flag), generator state structs, GC ownership (engine WasmGC owns object
  lifetime in wasm mode; MMTk never runs there; __del__/weakref
  divergences recorded).
- Leaf briefs: `leaves/03.01` .. `leaves/03.07` (7 leaves). Orchestrator
  Test Gate coverage matrix filled.
- Hello-world skeleton (crates were empty stubs):
  - `crates/ap-codegen-wasm`: `lower_module` (print-str-literal → WASI
    fd_write bootstrap; everything else explicit `CodegenError::Unsupported`)
    + 2 unit tests. Deps: ap-ir, wasm-encoder 0.263.
  - `crates/ap-host`: `run(bytes) -> RunResult{stdout,stderr,exit_code}`
    (wasmtime 47, WasmGC+EH flags explicit, WASI preview1 via
    wasmtime-wasi p1, MemoryOutputPipe capture) + 1 end-to-end test.
  - `cargo test -p ap-codegen-wasm -p ap-host`: see leaf-report / commit
    message when landed (design wave did not commit).

## Key decisions locked in the design wave (for orchestrator review)

| Topic | Decision | Where |
| --- | --- | --- |
| Value representation | uniformly boxed `(ref null $pyobj)`, no i31ref in v1 (ap-obj smi is i61) | design §1 |
| bool | $int singletons with `bool` type object (bool ⊂ int, S-01-9) | design §2 |
| Bigint | struct-based in-module; NOT a host import (portability, GC rooting, shared-source byte-exactness); orchestrator's "bignum import" surface narrowed accordingly | design §4, leaf 03.05 scope 3 |
| Exceptions | Wasm EH `try_table`/`throw`, tag payload `(ref $exception)`; wasmtime >= 47 pin (>= 37 behind `wasm_exceptions(true)`); no error-value fallback | design §0/§5 |
| GC ownership | engine's WasmGC collector owns object lifetime in wasm instances; MMTk never runs in wasm mode; `__del__` uncalled + `weakref` refused = tracked Class-X divergences | design §8 |
| Threading | wasmtime 47 REMOVED wasi-threads → v1 single-threaded + startup diagnostic; 03.06 spikes shared-everything-threads | design §9, leaf 03.06 |
| Generators | explicit $generator state struct + br_table state machine (p2w-cited design); no asyncify, no stack-switching in v1 | design §6 |

## Dispatch order (orchestrator.md, unchanged)

03.01 (spec 07 = the contract; unblocks T-12/T-13) → 03.02 → 03.03 →
03.04 sequential; 03.05 parallel from 03.03 onward; 03.06 then 03.07 last.


## 2026-11-07: leaf 03.02 SPLIT (orchestrator decision)

03.02 as briefed (values + arithmetic + control flow + calls + containers)
is multi-session work: byte-exact bigint limb routines, siphash13, compact
dict probing, and shortest-round-trip float repr must be emitted in-module
or as prelude Rust (spec-07 §6/§9) — an emitter restructuring, not one
leaf. Split: 03.02a (skeleton: type hierarchy, rt_init, GC-$str print),
03.02b (prelude runtime + value/arithmetic lowering), 03.02c (control
flow + calls + closures). 03.02a LANDED in private worktree
~/.git/worktrees/leaf-0302 @ 1b02ae8 (from 47be826), verified by the
orchestrator: 5/5 ap-codegen-wasm + 3/3 ap-host tests green incl.
end-to-end wasmtime hello-world; 18 named Unsupported sites, fail-closed.
NOT merged to trunk; merge with the 03.02b/c wave.
ENGINE FINDING carried into 03.02b scope: wasmtime 47 drops ACTIVE data
segments after instantiation — the emitter carries a passive duplicate
segment (index 1) for array.new_data. DEVIATION for 03.05 review:
aprt.abort_msg declared param is anyref instead of spec-07-41 (ref $str)
— wasmtime host-func matching cannot express the self-referential
concrete type; call sites still pass $str values.
