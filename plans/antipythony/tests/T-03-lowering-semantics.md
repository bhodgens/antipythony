# T-03 — Lowering semantics (gates implementation leaf 01.03)

Authorship: written from the spec pack ONLY (`docs/spec/01-values-and-objects.md`,
`docs/spec/02-control-flow.md`, `docs/spec/04-modules-imports.md`,
`docs/compat-contract.md`, `plans/antipythony/master.md`), per
`plans/antipythony/tests/README.md`. The author has not seen any implementation
source under `crates/`.

## Purpose

Gate leaf 01.03 (AST -> IR lowering) on SEMANTICS, specifically the part the spec
actually pins: the scope-resolution pass.

1. **Scope classification is spec-pinned.** Leaf 01.03 requires the lowering pass to
   emit per-function `cell` / `free` / `global` classification, and the spec clauses
   that determine those sets are MUST-level observable semantics:
   - S-02-1/2: static LEGB; any assignment anywhere in a body makes the name local to
     that function for the whole body.
   - S-02-3/4: `global` pins a name to module scope; `nonlocal` pins to the nearest
     ENCLOSING function scope (never module scope).
   - S-02-5: class bodies are NOT enclosing scopes; comprehensions (and generator
     expressions) have their OWN implicit scope (iteration var does not leak;
     FIRST iterable evaluated in the enclosing scope); `for` targets DO leak into the
     enclosing function scope.
   - S-02-12/13/14: an inner function referencing an enclosing variable captures the
     VARIABLE (cell), all closures over the same variable share ONE cell, cells
     outlive the defining frame.
   These four families make cell/free/global membership *derivable from the spec
   alone* for the 20 corpus programs — which is exactly what a test leaf must assert,
   because no CPython runtime emits IR (the spec, plus its CPython-observed anchors,
   is the source).
2. **Goldens to the depth the spec forces.** Each of the 20 corpus programs has a
   checked-in golden (`goldens/<program>.scope.json`) containing (a) the derived
   scope-classification table, and (b) the CPython 3.14.7 reference stdout/rc of the
   program (anchored semantics, cited per corpus rule). Full-tree IR serializations
   are NOT asserted where the spec does not pin node shapes — see CONTRACT-GAPS G-7.
3. **Determinism.** `dump(input)` twice -> byte-identical, for both the plain IR dump
   and the `--scopes` view (01.03 gate "all 20 goldens byte-stable"; compat-contract
   C-10 machine-stability principle; master plan 01 design "stability matters more
   than completeness").
4. **Effect-order preservation.** plan-01 design rule: "lowering must not drop or
   reorder operations with observable effects." Asserted as relative line order in
   the dump for a straight-line print sequence (see G-3 for why only relative order).
5. **No crash** on any corpus program: exit 0 for both dump views.

## Testability contract under test (from leaf 01.03's Expose/Gate)

- `ap_ir::lower(ast) -> Result<ir::Module, LowerError>` — the library seam (01.03
  Expose).
- Seam extension used by this suite: the `ap-ir-dump` bin established by the T-02
  contract, plus a `--scopes` flag printing one line per function-like scope:

  ```
  SCOPE name=<frame-name> cell=[a,b] free=[c] global=[d,e] [flags=[generator,...]]
  ```

  Names sorted; empty set = empty brackets. `<frame-name>` follows S-02-38
  (`<module>`, `<lambda>`, innermost `co_name` — methods print `m`/`__enter__`,
  not `C.m`). Comprehension scopes print `<listcomp>` (S-02-38 does not name
  comprehension frames — see G-5). The exact CLI flag, line format, and `flags`
  vocabulary are this leaf's seam choice — the leaf Expose pins only that the pass
  "emits per-function cell/free/global classification"; see G-1/G-2/G-6.
- Node-kind names for the plain dump inherit T-02's derivation (plan-01 design shape
  list Pascal-cased); not re-pinned here.

## Corpus (20 programs, `tests/t03_lowering_semantics/corpus/`)

Every program's header comment cites the clause ids it exercises (corpus rule,
tests/README.md). Reference stdout/rc were recorded by running each program under
`/opt/homebrew/bin/python3.14` (3.14.7) and are stored in the golden file.

| # | Program | Cites | What lowering must get right |
| --- | --- | --- | --- |
| 1 | `c01_late_bind_closure` | S-02-12, S-02-5, S-02-1 | comp var `i` is a cell in the comprehension scope; the lambdas' `i` is `free` — late binding yields `[2, 2, 2]` |
| 2 | `c02_shared_cell` | S-02-13, S-02-3, S-02-12 | ONE cell for `n` in `outer`; both `inc` (nonlocal write) and `read` (bare read) see `free=[n]` |
| 3 | `c03_nonlocal_counter` | S-02-3, S-02-12, S-02-14 | returned closure keeps its cell alive after `make_counter` returns |
| 4 | `c04_global_statement` | S-02-3 | `global x` -> `global=[x]`, NEVER cell/free |
| 5 | `c05_class_body_not_enclosing` | S-02-5, S-02-1 | method `m` must NOT have `x` free (class body is not an enclosing scope) — resolves to global level, NameError at runtime |
| 6 | `c06_comprehension_no_leak` | S-02-5 | comp scopes bind `v`/`q` only; iteration vars do not leak to module scope |
| 7 | `c07_for_var_leaks` | S-02-5, S-02-17 | `k` is a plain LOCAL of `f` (leaks within `f`; no cell, no free) |
| 8 | `c08_default_eval_at_def` | S-02-7 | default `[]` lowered into the MODULE-scope def-sequence, not `mk`'s body (shared mutable default -> `[1, 1] [1, 1]`) |
| 9 | `c09_args_kwargs_binding` | S-02-8 | `*args`/kwonly/`**kwargs` parameter kinds preserved (tuple/dict collect semantics observable) |
| 10 | `c10_kwonly_params` | S-02-8, S-02-10, S-02-3 | kwonly params after bare `*`; closure `cell` inside a kwonly function classified normally |
| 11 | `c11_generator_send_return` | S-02-20/21/22 | `g` flagged generator; `yield` node with send-value edge; `return x + 1` -> `StopIteration.value == 11` (anchor A02-05) |
| 12 | `c12_pep479` | S-02-23 | generator body's inner `StopIteration` -> `RuntimeError: generator raised StopIteration` w/ `__cause__` (anchor A02-06) |
| 13 | `c13_yield_from` | S-02-26 | `deleg` flagged yield-from; send relay + sub return-value propagation (`value 15`) |
| 14 | `c14_try_except_else_finally` | S-02-30/32/33, S-02-28 | handler var `e` scoped to handler only; bare `raise` re-raises ORIGINAL (rc 1, `ValueError: boom`); finally after handler |
| 15 | `c15_exception_chaining` | S-02-34/35 | `__context__` set on in-flight raise (`suppressed: False`); `raise X from Y` -> `__cause__`, `__suppress_context__ True` |
| 16 | `c16_with_protocol` | S-02-40/41 | nested `with` enter order a,swallow / exit order reverse, swallow suppresses, `exit:a:None` clean path |
| 17 | `c17_match_statements` | S-02-15, S-02-19, S-01-34 | case bindings (`first`, `rest`, `val`, `px`, `py`, `other`) are locals of `classify`; guard lowered after pattern |
| 18 | `c18_fstrings` | S-01-16 | interpolation with `str()` default, `!r/!s/!a` conversions, `:format_spec`; `ascii` appears as a module-scope ref |
| 19 | `c19_decorator_order` | S-02-7, S-02-12 | def-time order: defaults -> decorators innermost-first -> bind name; `deco.cell=[fn]`, `wrapper.free=[fn]`; observed calls `['d1', 'deco:g', 'deco:wrapper', 'call:wrapper', 'call:g']` |
| 20 | `c20_import_forms` | S-04-06/07/08 | import statement binding effects: dotted import binds ONLY top name, `from` binds ONLY the imported/aliased name |

## Scope-classification derivations (the golden source = spec rules)

`cell` = names this scope binds that an inner scope captures (S-02-12/13);
`free` = names resolved in an ENCLOSING function/comprehension scope (S-02-1/3,
nonlocal or bare read); `global` = names resolved at module-or-builtin level
(S-02-1 LEGB Global/Builtin level, S-02-3 `global` stmt, S-02-5 class-body skip;
convention pinned in G-2). Scope name may repeat within a program; comparison is
by multiset (see G-5).

| Program | Scope | cell | free | global | Derivation |
| --- | --- | --- | --- | --- | --- |
| c01 | `<module>` | — | — | print, range | `range(3)` is the FIRST iterable -> evaluated in enclosing (module) scope, S-02-5 |
| c01 | `<listcomp>` #1 | i | — | — | comp binds its own scope, S-02-5; `i` captured by the lambda -> cell, S-02-12 |
| c01 | `<lambda>` | — | i | — | bare read of enclosing comp var, no declaration needed, S-02-6/12 |
| c01 | `<listcomp>` #2 | — | — | fns | binds `fn`; `fns` is first iterable -> evaluated in module scope, read as a global ref, S-02-5 |
| c02 | `<module>` | — | — | print | |
| c02 | `outer` | n | — | — | ONE shared cell for `n` (S-02-13); `inc` nonlocal-writes and `read` bare-reads the SAME variable |
| c02 | `inc` | — | n | — | `nonlocal n` -> nearest enclosing function scope, S-02-3 |
| c02 | `read` | — | n | — | read-only closure ref, no declaration, S-02-3/6 |
| c03 | `<module>` | — | — | print | |
| c03 | `make_counter` | count | — | — | S-02-12/13: `count` captured by `counter` |
| c03 | `counter` | — | count | — | `nonlocal count`, S-02-3; cell survives return (S-02-14) |
| c04 | `<module>` | — | — | print | |
| c04 | `bump` | — | — | x | `global x`: ALL uses refer to module global, never cell, S-02-3 |
| c05 | `<module>` | — | — | NameError, print | |
| c05 | `C` | — | — | — | class body IS a scope (bindings exist, `x=10`) but is NOT an enclosing scope, S-02-5 |
| c05 | `m` | — | — | **x** | lookup skips the class body (S-02-5), no enclosing function, -> Global level; must NOT be `free=[x]` — that would contradict the observed NameError |
| c06 | `<module>` | — | — | print | |
| c06 | `<listcomp>` #1 | — | — | xs | binds `v` only; `xs` is first iterable -> module scope, S-02-5; no leak to module scope |
| c06 | `<listcomp>` #2 | — | — | ys | binds `q` only; `ys` first iterable -> module scope |
| c07 | `<module>` | — | — | print | |
| c07 | `f` | — | — | range | `k` is a plain local of `f` and leaks within `f` (S-02-5: `k == 2` after loop) — not classified (locals aren't a dump column) |
| c08 | `<module>` | — | — | print | default `[]` evaluated ONCE at def time IN module scope, S-02-7 — if it were lowered into `mk`'s body the `[1, 1] [1, 1]` anchor breaks |
| c08 | `mk` | — | — | — | body reads `seq` (parameter, local) |
| c09 | `<module>` | — | — | print | |
| c09 | `f` | — | — | sorted | params a,b,args,c,d,kw all local, S-02-8 |
| c10 | `<module>` | — | — | print | |
| c10 | `f` | cell | — | — | `inner` reads `cell` -> cell in `f`, S-02-12 |
| c10 | `inner` | — | cell | — | bare closure read |
| c11 | `<module>` | — | — | StopIteration, next, print | |
| c11 | `g` [generator] | — | — | — | body contains `yield` -> generator function, S-02-20; `x` local receiving `send` value, S-02-21; `return x+1` -> `StopIteration.value`, S-02-22 |
| c12 | `<module>` | — | — | RuntimeError, list, print, type | |
| c12 | `g` [generator] | — | — | iter, next | `iter`/`next` are builtin-level refs; inner StopIteration transformation is a lowering/runtime obligation, S-02-23 |
| c13 | `<module>` | — | — | StopIteration, next, print | |
| c13 | `inner` [generator] | — | — | — | `v` local (send relay target, S-02-26) |
| c13 | `deleg` [generator, yield-from] | — | — | inner | `inner` is module-level -> Global level (S-02-1), NOT `free` |
| c14 | `<module>` | — | — | print | |
| c14 | `h` | — | — | TypeError, ValueError, str | `e` scoped to the handler only, deleted at handler exit, S-02-30 — a post-handler read of `e` must verify as local-unbound (01.04), not resolve global; `first` local (S-02-2) |
| c15 | `<module>` | — | — | ValueError, ZeroDivisionError, int, print, type | no nested functions; chaining is a raise/except lowering obligation, S-02-34 |
| c16 | `<module>` | — | — | ValueError, print | |
| c16 | `CM` | — | — | — | class body scope (see G-4) |
| c16 | `__init__` | — | — | — | |
| c16 | `__enter__` | — | — | **log** | class body NOT enclosing scope, S-02-5 -> `log` is a global ref, never free |
| c16 | `__exit__` | — | — | log, str | |
| c17 | `<module>` | — | — | print | |
| c17 | `classify` | — | — | Point | `Point` defined AFTER `classify` textually — module-level ref resolved at CALL time (S-02-1 Global level); case vars (`first`, `rest`, `val`, `px`, `py`, `other`) are locals of `classify`, S-02-19 unpacking |
| c17 | `Point` | — | — | — | class body scope |
| c17 | `__init__` | — | — | — | |
| c18 | `<module>` | — | — | ascii, print | f-strings interpolate at module scope; `!a` calls `ascii`, S-01-16; f-string literal parts are data, not name refs |
| c19 | `<module>` | — | — | print | def-time sequence: `mark(1)` (default), then `@deco @deco` innermost-first — all lowered into module scope, S-02-7 |
| c19 | `deco` | fn | — | calls | `wrapper` reads `fn` -> cell, S-02-12; `calls` is module-level -> Global |
| c19 | `wrapper` | — | fn | calls | params `a`/`kw` local (it is `def wrapper(*a, **kw)`, S-02-8) |
| c19 | `mark` | — | — | calls, str | |
| c19 | `g` | — | — | — | body `return seq`; `seq` local |
| c20 | `<module>` | — | — | dir, print, sorted, type | import statements bind `os`/`p`/`collections` as module NAMES (S-04-06/07/08) — locals of `<module>`, not global refs; `p.join`/`collections.OrderedDict` are attribute reads through those bindings |

## Per-test citations

| Test | Input | Assertion | Cites |
| --- | --- | --- | --- |
| `test_scope_classification_matches_golden` (×20 params) | corpus program | `--scopes` dump multiset == golden (name, cell, free, global) exactly; golden `flags` (when present) subset of dump flags | S-02-1/2/3/5/12/13/14; S-02-7 (def-time scope of defaults); S-02-20/26 (flags); S-01-16; S-02-38 (names); leaf 01.03 Gate "scope classification matches spec clauses" |
| `test_dump_is_byte_deterministic` (×20 params) | corpus program, dumped twice (plain + `--scopes`) | byte-identical, non-empty | 01.03 Gate "all 20 goldens byte-stable"; C-10 machine-stability; master plan-01 design |
| `test_all_corpus_programs_lower_without_crash` | all 20 programs | exit 0 + non-empty dump for both views | leaf 01.03 Expose full-v1-subset lowering; plan-01 "no construct crashes" (T-02 principle carried over) |
| `test_lowering_preserves_observable_effect_order` | straight-line `print("A");print("B");print("C")` | first dump line containing each literal appears in source order | plan-01 Semantics preservation rule ("must not drop or reorder operations with observable effects"); spec-02 preamble ("every construct below defines the observable order tests may rely on") |

Golden-file self-consistency (citation match between corpus header and golden,
reference stdout/rc present, scopes non-empty) is checked inside
`test_scope_classification_matches_golden` before the seam call, and was verified
at authoring time by the generator run (CPython 3.14.7 outputs recorded, cited
sets matched against headers).

## Run command

```
pytest tests/t03_lowering_semantics/ -v
```

From the repo root. The suite invokes `cargo run --quiet --bin ap-ir-dump --
[--scopes] <file>`; override with env `AP_IR_DUMP="<cmdline>"` (same seam as T-02).
Python: `/opt/homebrew/bin/python3.14 -m pytest tests/t03_lowering_semantics -q`.

## xfail policy

`ap_ir::lower` and the `--scopes` seam do not exist yet (leaf 01.03 not started).
Every test is marked `xfail(strict=False, reason="awaits 01.03 ap_ir::lower +
ap-ir-dump --scopes seam")`. Until 01.03 lands: suite is all-xfail, zero hard
failures/errors — GREEN for this leaf's own gate. After landing: xpass reported;
failures are 01.03 gate failures — fix the implementation, never weaken the test
(tests/README.md corpus rule; AGENTS.md "corpus only grows").

## CONTRACT-GAPS

- **G-1: the `--scopes` serialization surface is a seam choice, not spec text.**
  Leaf 01.03's Expose pins `ap_ir::lower(ast)` and that the pass "emits
  per-function cell/free/global classification", but nothing in the spec pack pins
  HOW that classification is rendered observably. This suite invents the minimal
  machine-stable rendering (one `SCOPE name= cell= free= global= [flags=]` line per
  scope, sets sorted — sortedness per the C-10 machine-stability principle).
  Resolution: either fold this format into the dump contract addendum (T-02 G-1
  territory) or have 01.03 expose an equivalent query; the goldens then regenerate
  mechanically from the derivation table above (the SEMANTIC sets do not change,
  only their rendering).
- **G-2: the `global` column's membership convention is unpinned.** S-02-12/13 pin
  `cell`/`free` closure membership exactly; they do not say whether `global=[...]`
  lists (a) only `global`-declared names, (b) names resolving to module level, or
  (c) names resolving to module OR builtin level. This suite takes the strictest
  reading — (c) — because S-02-1 makes LEGB's Global→Builtin fallthrough observable
  behavior (an unbound global raises NameError; a builtin resolves) and 01.04's
  unbound-local verifier needs all three levels distinguished. If the implementer
  chooses (a)/(b), every `cell`/`free` set here still holds unchanged; only the
  `global` column shrinks — goldens must be regenerated against the pinned
  convention, and this gap closed in the spec pack, NOT by editing the derivation
  logic.
- **G-3: non-scope node shapes are deliberately unasserted.** The spec does not pin
  how try/finally (S-02-33), with (S-02-40/41), match (pattern-case ordering,
  S-02-15/19), generators (S-02-20..26), or f-strings (S-01-16) lower into node
  trees. Exact-tree goldens for those shapes would encode implementation choices,
  so this leaf asserts only: (i) scope classification where derivable, (ii) dump
  determinism, (iii) effect-order (as relative line order, valid only if the dump
  is an in-order traversal — an unpinned presentation choice, listed here), and
  (iv) CPython anchor outputs recorded in goldens as the future 02.x differential
  target. Every unpinned shape decision (e.g. try/finally as a node with a
  cleanup-child vs duplicated epilogue) is intentionally NOT written down as an
  expectation.
- **G-4: class-body scopes in the dump.** S-02-5 makes class-body scopes exist
  (bindings land there) but does not say they appear in a scope dump. Goldens here
  EMIT class-body scopes (`C`, `CM`, `Point`) — derivable, since `x = 10` inside
  `class C:` must bind somewhere distinct from module locals. If 01.03 omits class
  scopes from the dump, the goldens lose those rows (no cell/free/global assertion
  for them changes).
- **G-5: comprehension scope names + duplicate names.** S-02-38 pins function,
  lambda and `<module>` frame names; it is silent on comprehension frames (chosen
  here: `<listcomp>`, mirroring CPython runtime frames, which comprehension scopes
  can only produce in tracebacks absent from the v1 corpus). Scope names may repeat
  in one program (two `<listcomp>` in c01, two in c06, `<lambda>` once), so
  classification is compared as a MULTISET of (name, cell, free, global) tuples —
  an unpinned identity decision, listed here.
- **G-6: flag vocabulary.** "is a generator" (S-02-20) and "has yield-from"
  (S-02-26) are pinned OBSERVABLE facts the IR must carry, but no field name or
  token vocabulary is pinned. Goldens use `flags=[generator]`/`[generator,
  yield-from]`; tests check golden flags are a SUBSET of dump flags, so additional
  flags never fail, but a missing marker does (the strictest reading).
- **G-7: "checked-in IR golden serializations" depth.** The leaf gate says 20
  programs -> checked-in IR goldens. Until the full dump vocabulary is pinned
  (T-02 G-1 addendum), goldens here pin the spec-pinned slice (scope table +
  CPython reference output + cite manifest) and leave the full-tree dump to a
  promotion step: once vocabulary is pinned, freeze each program's complete dump
  as `goldens/<program>.ir.txt` and add a byte-equality test — this leaf's
  determinism test already covers the stability half of that gate.
- **G-8: c14 raises at runtime by design.** The bare-raise program exits rc 1 with
  a `ValueError: boom` traceback; the golden records that (path-normalized per
  spec-02 convention) as future 02.x differential data. T-03 asserts only the
  lowering-side facts for it (scope table, dump stability, no crash) — runtime
  traceback byte-exactness is the conformance runner's job, not this suite's.

## Protocol notes

- Tests commit BEFORE 01.03 completes (README §Protocol 3); gate-time runner is
  an agent that wrote neither these tests nor the lowering code (§Protocol 4).
- Reference outputs measured on `/opt/homebrew/bin/python3.14` = 3.14.7 (same
  provisional-status caveat as the 00-foundation review items; re-verify on
  3.14.0 pending).
- Spec contradictions found: none. Gaps G-1..G-8 routed per README §Protocol 5 —
  each needs a spec/contract PR, never a silent test edit.
