# Plan 02 progress log

## Pon study - COMPLETE (committed 1237c75)
- docs/pon-study-lowering.md (185 L) + docs/pon-study-tier1.md (208 L), clone
  d0a17177, all cited paths verified, zero verbatim copies.
- KEY ADOPTABLES (lowering): generator state-machine transform (suspend
  markers -> dense states + cross-block spill), finally-trampoline edge
  discipline, __classcell__ protocol, match eager-partial-bind, ruff f-string
  parts-not-elements gotcha (POSSIBLE LIVE BUG in our f-string lowering -
  needs a differential test, queue as 01.03 follow-up).
- ARCHITECTURAL FORK IDENTIFIED: pon lowers straight to flat CFG IR with
  semantics resolved at lower time; our ap-ir is structured stmt-tree (CFG
  lowering deferred to codegen). Decision needed BEFORE 02.05: stay structured
  (codegen does CFG) vs adopt flat-CFG lowering. The 02.05 implementer reads
  the note and decides; default = stay structured, document why.
- TIER-1 TOP-5 RISKS (from note): (1) precise GC roots across native stack -
  pon shim adaptable but MMTk contract needs design; (2) OSR with unboxed
  state - pon has no answer, redesign required; (3) true deopt materialization
  beyond cold-twin - nothing in pon, highest risk; (4) IC version-bump
  completeness audit; (5) feedback-slot stability across lowering stages.
  These shape 02.05's leaf - it should be SPLIT if it overruns (per master
  dispatch note).

## Leaf 02.01 - ap-obj core values - DISPATCHED (running)
- monty (MIT) as adaptation donor w/ attribution; smi+bigint choice documented;
  CPython-differential tests; 02.03/02.04 constraints pre-declared.

## Test leaf T-05 - DISPATCHED (running, separate authorship)
- Probing reference CPython for pinned bytes is allowed (reference, not impl).

## F-string bug hunt (parallel track) - COMPLETE (committed 0992ee9)
- BUG-1 CONFIRMED (medium): `{x=}` debug forms - debug_text dropped at
  lowering (f"{x=}" -> IR f"{x}"); implied-!r rule also lost. Fix: add
  debug_text to FStrPart::Interp + read it in lower_interp_element + pon's
  implied-r rule. Repros f17-f21.
- pon parts-not-elements: NOT a bug in us (verified).
- DUMP-1 cosmetic: multiline Lit renders raw newlines (dump-only).
- FINDING-3 (bigger than f-strings): verify.rs walrus binds are indefinite
  (scope.bind, not bind_definite) - every walrus-then-read false-flags
  NameError. One-line fix queued with BUG-1.
- Battery: 35 cases, zero panics; corpus slice 35+105 files pinned from live
  3.14.7; gate suite 107/107 green.
- NOTE: agent believed s005 open - stale (corrected in f0a4c22); AGENTS.md
  wording misled it.

## Fix leaf (BUG-1 + FINDING-3) - COMPLETE (committed 075b453)
- debug_text: Option<String> on FStrPart::Interp; lowering reads ruff
  DebugText::as_str() (text() doesn't exist in 0.0.4); pon implied-r rule
  implemented; dump renders debug verbatim (f"{x=!r}" / f"{x=:>10}").
- Walrus: bind_definite; comp-IF walrus leaks (iteration targets don't);
  walrus->del->read still flags. +8 tests (72 total), fstrings gate 107/107.
- Verified firsthand: f17/f19 dumps correct; suites green.

## QUEUE (updated)
1. 02.02 tier-0 executor + T-06 after 02.01 + T-05 land (running).
2. 02.03 classes/protocols + 02.04 GC after 02.02.
3. 02.05 tier-1 (SPLIT CANDIDATE - top-5 risks in pon study note).
