# Plan 10 - C-Extension Compatibility (HAIL MARY - isolated stretch)

Goal: let existing CPython C extensions run against antipythony. This is the single
hardest problem in the space (GraalPy: ~a decade, C-API only). It is ISOLATED by
design: nothing in Plans 00-04 may depend on any leaf here, and failure here blocks
nothing.

## Position

v1 policy (locked): extension imports fail with a clear compile-time diagnostic.
v2 policy (this plan's target): "rebuild-required" - a C-API subset sufficient for
popular extensions to recompile against antipythony. Binary wheels built for
CPython NEVER load (different ABI, different object layout) - the honest boundary
GraalPy and kohebi both document.

Prior art to study (research leaves, not code donors):
- componentize-py (Apache-2.0): how real CPython is shipped INTO wasm - useful for
  the strategy "extensions keep expecting libpython, we provide a libpython-like
  surface."
- GraalPy docs + changelog: the C-API emulation hit list, what they refused.
- RustPython: their attempts at C-extension support and why they stalled.
- CPython internals: object layout, refcount expectations, stable ABI (PEP 384/659)
  as the surface we must mimic or explicitly reject.

## Leaves

### 10.01 - Survey and feasibility report (research only)
Read the prior art above. Produce `docs/ce-compat-survey.md`: which C-API surface
the top-50 PyPI native extensions actually use (measure, do not guess - script the
symbol scan against real wheels); which subsets are feasible; the recommended
strategy (full C-API emulation in Rust vs a new extension ABI + source-shim
tool). REQUIRED CASE STUDY (master tier table): pydantic-core - the single
recurring blocker for the T3 (hermes-agent) and T4 (fastapi) ecosystem targets;
it is itself a Rust crate built as a CPython extension, so its feasibility here
is the sharpest early signal for whether T3/T4 are reachable. Gate: main-model
review; explicit recommendation with a kill-criterion.

### 10.02 - Prototype: 3 C-API functions (spike)
After 10.01 recommends proceed: implement the smallest C-API surface that lets ONE
trivial hand-written extension compile and run against antipythony native. This is
a throwaway spike measuring difficulty density per API function. Gate: the spike
report states hours-per-API-function; if the extrapolated top-50 cost exceeds the
threshold in the survey, STOP and report (the kill-criterion executes here).

### 10.03 - Extension build tool
If 10.02 passes: `antipythony build-ext` - compile a C extension source against our
headers (ap-obj-backed C-API shim), link, load. One extension family target
(cryptography or a simple C accelerator). Gate: chosen extension's own test suite
passes under antipythony native.

### 10.04 - Wasm extensions (far stretch)
Extensions compiled to wasm + host-import shims. Feasibility note only unless 10.03
landed cleanly. Gate: one extension running in wasm mode, or a documented
not-in-v2 verdict.

## Kill criteria (write these into the survey, execute without sentiment)
- Top-50 surface area implies > 2 person-years after the 10.02 density measurement.
- Any API requires CPython object layout byte-compatibility we cannot provide
  without abandoning the GC design.
- Conformance regressions in Plans 02-03 caused by C-API accommodation.

## Dependencies
None for 10.01 (can run in parallel with Plan 01+). 10.02+ require 02.05 stable.

## Dispatch notes
10.01 any time. The isolation rule is absolute: no other plan imports from,
links to, or waits on this one. Its crates live under `contrib/ce-compat/` until
10.02 passes its gate, then graduate to the workspace.

## Test Gate

<!-- Tests-first gate (hierarchical-planning skill v1.3.0, patched 2026-10-24).
     Coverage matrix derived mechanically from leaf task headers; the control
     agent (orchestrator, in-session) refines rows and fills test names at
     Phase 0 dispatch. No implementation leaf dispatches until every row is
     VALIDATED. Delta protocol: lesser of plan/tests updates to the greater.
     Recipe: skill hierarchical-planning -> references/tests-first-gate.md -->

N/A — no leaf documents found alongside this orchestrator. If this tree
gains leaves, build the coverage matrix before dispatching them.
