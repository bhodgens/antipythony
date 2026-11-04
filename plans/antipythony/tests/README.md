# Test Tree - plans/antipythony/tests/

Separate-authorship test track. Test documents here are written by DIFFERENT agents,
in a separate pass, from the spec pack ONLY. The implementing agent for a leaf never
writes that leaf's tests.

## Inputs test agents receive

1. `docs/spec/01..06` (spec pack; a future `07-wasm-repr` doc lands with leaf
   03.01 and does not exist yet)
2. `docs/compat-contract.md`
3. `plans/antipythony/master.md` (contracts section)
4. The corresponding leaf document's "Expose" and "Gate" sections.

## Inputs test agents NEVER receive

Implementation source, implementation git history, implementation PRs.

## Layout in the repo

```
tests/
  corpus/           # shared differential corpus (.py scripts), grows monotonically
  integration/      # cross-crate e2e (CLI verbs, build outputs)
  01-parse/         # per-leaf unit tests live near this shape, per plan tree refs
docs/spec/          # the only semantics source
```

Corpus rules:
- Every corpus case cites spec clause ids it exercises (header comment).
- Corpus cases are NEVER deleted or weakened to make a gate pass; changes require
  master.md integration review.
- Corpus grows in slices: values, control-flow, classes, exceptions, generators,
  stdlib-v1, threading, performance-probes.

## Test leaves (dispatch after 00.05 lands)

| Test leaf | Gates implementation leaf | Written from |
| --- | --- | --- |
| T-01 parse diagnostics | 01.01 | spec 04 + contract |
| T-02 IR shape | 01.02 | spec 01-02 |
| T-03 lowering semantics | 01.03 | spec 01-04 |
| T-04 verify (unbound-local) | 01.04 | spec 02 clause set |
| T-05 values | 02.01 | spec 01 |
| T-06 classes | 02.03 | spec 03 |
| T-07 gc/finalizer | 02.04 | spec 01 + contract divergence |
| T-08 tier-1 transparency | 02.05 | contract (invisible-except-timing rule) |
| T-09 binary conformance | 02.06 | contract + corpus slices |
| T-10 threading | 02.07 | spec 02 + threading corpus |
| T-11 stdlib-v1 | 02.08 | spec 05 |
| T-12 wasm parity | 03.02-03.04 | spec 06 (byte-identical rule) |
| T-13 host/wasi | 03.05 | spec 06 |
| T-14 cli/cache | 04.01-04.02 | leaf gate sections |
| T-15 release/install | 04.03 | leaf gate sections |
| T-16 hybrid artifact | 04.05 | leaf gate sections |

## Protocol

1. Test leaf dispatched to a fresh agent with ONLY the inputs above.
2. Agent writes tests (pytest/rust test harness as appropriate to the gate),
   including expected-fail skeletons where the implementation does not exist yet.
3. Tests are committed BEFORE the implementation leaf they gate completes.
4. Implementation leaf completion requires: its test leaf run by the orchestrator,
   all green, by an agent that did not write the tests and did not write the code.
5. A test agent finding a spec contradiction files it back to the spec pack
   (PR against docs/spec with clause-id reasoning) - never silently picks semantics.
