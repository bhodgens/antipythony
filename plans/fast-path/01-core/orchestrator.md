# FAST-PATH — orchestrator (01-core)

Core execution semantics: control flow, calls/attrs, exceptions.
Order matters: control flow first (biggest walk-floor win, no
exception-table complexity), calls second, exceptions last (hardest
byte-exactness surface — traceback line attribution).

## Child index

| leaf | what | depends |
|---|---|---|
| 01-control-flow.md | While/For lowering + Jump/BoolOp/Break/Continue + execution | 00-infra merged |
| 02-calls-and-attrs.md | Call/LoadAttr/Subscript opcodes + comprehension lowering | 01 merged |
| 03-exceptions.md | exception-table Try/except/finally/raise + line attribution | 01 + 02 merged |

## Dispatch protocol

1. Sequential: 01 -> review/merge -> 02 -> review/merge -> 03.
2. Each leaf extends fix_bytecode.py (the three-way differential) —
   the gate grows with coverage; earlier coverage must stay green.

## Test Gate

fix_bytecode.py three-way identical (treewalk/bytecode/CPython) over
the full coverage ladder; s016 traceback-line family; standard
battery in both toggle states.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-control-flow.md | DONE on branch tier1-fp-controlflow (session record NEXT-FP-CONTROLFLOW.md; gates green both toggles; merge pending orchestrator review) |
| 02-calls-and-attrs.md | PENDING (partially pre-empted: LoadAttrCall fused opcode + AugOp landed with 01) |
| 03-exceptions.md | DONE on branch tier1-fp-exceptions (session record NEXT-FP-EXCEPTIONS.md; fix_bytecode 13/13 three-way green incl. the new PROBE_EXCEPTIONS; conform 50/50+48/48 both toggles; walker return-in-finally bug fixed as a carry-out; merge pending orchestrator review) |
