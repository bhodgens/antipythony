# T-02 — IR shape (gates implementation leaf 01.02)

Authorship: written from the spec pack ONLY (`docs/spec/01-values-and-objects.md`,
`docs/spec/02-control-flow.md`, `docs/compat-contract.md`,
`plans/antipythony/master.md`), per `plans/antipythony/tests/README.md`. The author
has not seen any implementation source under `crates/`.

## Purpose

Gate leaf 01.02 (ap-ir data structures + Builder) on SHAPE, not semantics:

1. Every Expr/Stmt construct named in spec 01 and spec 02 is representable in the
   IR — observable via the contract bin `ap-ir-dump <file>`, which parses the given
   Python file and prints an indented IR dump, one node per line, each line of the
   form `node-kind` + optional ` detail`.
2. Parse-tree structure is preserved where spec makes it observable: operator
   chaining produces the number of operator nodes Python's grammar requires.
3. Dumps are deterministic: identical input → byte-identical output (required for
   the 01.03 golden tests; master plan 01 design: "stable PartialEq on all nodes
   (needed for golden tests)"; same machine-stability principle as compat-contract
   C-10 — diagnostics/serializations tests rely on MUST NOT vary run to run).
4. No spec-01/02 construct crashes the dumper (exit 0 on parse+dump).

## Testability contract under test (from leaf 01.02's Expose/Gate)

- Bin `ap-ir-dump <file>`: parse the Python file, print the IR dump, exit 0.
- Nodes exist for every Expr/Stmt kind in `docs/spec/01` + `docs/spec/02`.
- Node-kind names: the contract example pins `BinOp` (PascalCase of the plan-01
  design shape list: `binop, compare, call, lambda, comprehension, yield, await,
  fstring`; `assign, if, while, for, with, try, match, ...`). Tests apply the same
  mapping to the remaining kinds (Compare, Call, Lambda, Comprehension, Yield,
  Await, If, While, For, Try, Match, With, ClassDef, FunctionDef). See
  CONTRACT-GAPS G-1.

## Per-test citations

| Test | Program shape | Assertion | Cites |
| --- | --- | --- | --- |
| `test_binop_single` | `x = a + b` | exactly one line `^BinOp` | spec 01 §1 (binary arithmetic ops, S-01-1); plan-01 Expr list |
| `test_compare_single` | `b = a < c` | exactly one line `^Compare` | S-01-34 (ordering comparisons are real operations the IR must represent) |
| `test_compare_chain` | `b = a < c < d` | ≥1 line `^Compare` (chain may be one node or a pair chain — unpinned, see G-1) | S-01-34 (chained comparisons evaluate each operand at most once, short-circuit) |
| `test_call_kwargs` | `f(a, k=1)` | exactly one line `^Call` | S-02-9 (call-site positional/keyword unpacking, left-to-right evaluation) |
| `test_lambda` | `g = lambda x: x` | exactly one line `^Lambda` | S-02-11 (lambda body is one expression); S-02-38 (`<lambda>` frame name — lambda is a first-class function node) |
| `test_comprehension` | `ys = [i for i in xs]` | exactly one line `^Comprehension` | S-02-5 (comprehensions have their OWN implicit scope — distinct IR node required) |
| `test_yield` | `def g():` / `    yield 1` | exactly one line `^Yield` | S-02-20/21 (generator functions, yield suspends) |
| `test_await` | `async def f():` / `    await g()` | exactly one line `^Await` | plan-01 Expr list names `await`; ecosystem tier T1 (scrapy) requires async shapes |
| `test_fstring_beyond_literals` | `s = f"a{x}b"` | ≥1 line whose kind is NOT a literal part (see G-2) | S-01-16 (f-string interpolation with conversion/format-spec clauses) |
| `test_if` | `if a:` / `    pass` | exactly one line `^If` | S-02-15 |
| `test_while` | `while a:` / `    pass` | exactly one line `^While` | S-02-16 (while/else; cond tested before each iteration) |
| `test_for` | `for i in xs:` / `    pass` | exactly one line `^For` | S-02-17 (iter obtained once, `__next__` protocol) |
| `test_try_except_finally` | try/except ValueError/finally | exactly one line `^Try` | S-02-30/32/33 |
| `test_match` | `match x:` / `    case 1:` / `        pass` | exactly one line `^Match` | plan-01 Stmt list names `match`; Python 3.14 target grammar |
| `test_with` | `with cm:` / `    pass` | exactly one line `^With` | S-02-40/41 (enter/exit protocol, nesting order) |
| `test_class_def` | `class C:` / `    pass` | exactly one line `^ClassDef` | S-02-5 (class bodies; spec 03 preview — shape leaf only needs the node) |
| `test_function_def_defaults_kwonly` | `def f(a, b=2, *, c):` / `    pass` | exactly one line `^FunctionDef` | S-02-7/8 (defaults evaluated at def time; parameter kinds incl. kwonly) |
| `test_binop_chain_count` | `x = a + b + c` | exactly TWO lines `^BinOp` (left-assoc tree `(a+b)+c`) | S-01-2 (operator associativity is pinned observable semantics: `**` right-assoc `3 ** 3 ** 2` == 19683 — the grammar's associativity MUST be represented in the IR tree, so chained `+` yields two nodes, not one folded node) |
| `test_dump_deterministic` | any fixture program | two invocations → byte-identical stdout | master plan 01 design "stability matters more than completeness"; 01.02 Expose "stable PartialEq on all nodes (needed for golden tests)"; C-10 machine-stability principle |
| `test_all_constructs_no_crash` | one program using every spec-01/02 construct listed above | exit code 0, non-empty dump | plan-01 01.02 Gate ("builder constructs all corpus constructs") |

## Run command

```
pytest tests/t02_ir_shape/ -v
```

From the repo root. The suite invokes the contract bin via `cargo run --quiet
--bin ap-ir-dump -- <file>` from the repo root; override with env
`AP_IR_DUMP="<cmdline>"` to point at a prebuilt binary.

## xfail policy

The `ap-ir-dump` bin does not exist yet (leaf 01.02 in progress). Every test is
marked `xfail(strict=False, reason="awaits 01.02 ap-ir-dump bin")`. Consequences:

- Until 01.02 lands: suite shows all-xfail, zero hard failures — this is GREEN for
  the test leaf's own gate.
- After 01.02 lands: tests run for real; xpass is expected and reported, failures
  are gate failures for 01.02 (fix the implementation, never the test, per
  AGENTS.md "corpus only grows").
- `strict=False` is deliberate: the transition xfail→xpass must not be an error,
  and per-test shape mismatches after landing must show as real FAILURES, not
  suite-level noise.

## CONTRACT-GAPS

- **G-1: IR dump node vocabulary unpinned in spec.** Neither spec 01 nor spec 02
  names IR node kinds (they pin observable program semantics, not compiler
  internals). The kind names used here are derived from the plan-01 design shape
  list plus the contract's own `BinOp` example. Resolution: a docs/spec addendum
  (or the 01.02 exhaustiveness matrix referenced from it) pins the full dump
  vocabulary. Until then, tests assert the derived names exactly, and two places
  stay deliberately loose: compare chains (≥1 Compare line — one flattened node or
  a chain of nodes are both faithful to S-01-34) and the FunctionDef argument
  detail (kind count only, no per-arg node assertions).
- **G-2: f-string IR vocabulary unpinned.** S-01-16 pins f-string SEMANTICS
  (interpolation, `!r/!s/!a`, `:format_spec`, debug form) but no dump node kind
  (e.g. `Interp`/`FormatSpec`) is named anywhere in the spec pack. Until the
  addendum lands, `test_fstring_beyond_literals` asserts only that SOME
  non-literal-part node line appears for an f-string (i.e. the interpolating
  structure exists beyond the literal string parts), not a specific kind name.
- **G-3: dump child-order/indentation semantics unpinned.** The contract says
  "indented tree" but does not pin child ordering or indent width, so structural
  side (left- vs right-nesting) of operator chains is NOT assertable; only node
  COUNTS are (hence `test_binop_chain_count` counts, it does not compare
  indentation).
