# T-06 — classes + exception hierarchy (gates implementation leaf 02.03 ap-obj classes/protocols)

Authorship: written from the spec pack ONLY (`docs/spec/03-classes.md`,
`docs/spec/02-control-flow.md` §6 for the exception-hierarchy cross-cites,
`docs/spec/06-observable-behavior.md`, `docs/compat-contract.md`,
`plans/antipythany/master.md`, `plans/antipythany/02-native-backend/orchestrator.md`
02.03 Expose/Gate), per `plans/antipythany/tests/README.md`. The author has not
read any implementation source under `crates/`.

## Purpose

Gate leaf 02.03 (ap-obj: type objects, MRO C3, attribute lookup, descriptors,
special-method dispatch, metaclass hooks) with a differential corpus whose every
byte is pinned by CPython 3.14.7. The corpus exercises spec 03's MUST clauses:
instantiation order and errors (§3.2), attribute lookup priority including
data-vs-non-data descriptors (§3.3), `__slots__` messages (§3.4), C3 MRO worked
examples E1/E2/E3 and both rejection modes (§3.5), `super()` validation and the
diamond chase (§3.6), descriptor kinds (§3.7), dunder-bypass-instance-dict and
operator/reflected/augmented dispatch (§3.8–3.9), the metaclass hook pipeline
(§3.10), inherited-method binding (§3.11), `isinstance`/`issubclass` (§3.12), and
the exception class hierarchy + raise/except class rules S-03-32/33/34 (§3.13).

Two halves:

1. **Fixture re-verification (green today).** Each `.py` under
   `tests/corpus/classes/` and `tests/corpus/exceptions/` is re-run under the
   reference CPython and its stdout/stderr/exit bytes are diffed against
   recorded `expected/` files (the corpus is self-validating: no implementation
   needed). Header-citation, clause-coverage, determinism, and
   nondeterministic-address-leak checks ride along.
2. **ap-conform gate (xfail, strict=False).** The 02.03 gate — "corpus
   classes/exceptions slices green under tier-0" — expressed as
   `ap-conform run --corpus <slice glob> --impl <cmd> --tier 0` per the 00.04
   CLI contract, once per slice and once per case. Neither the seam nor ap-obj
   exists yet; all 50 gate cases XFAIL.

## Testability contract under test (from leaf 02.03's Expose/Gate)

- Expose: ap-obj surface — type objects, C3 MRO, attribute-lookup order per
  spec 03, descriptors/properties, special-method dispatch, metaclass hooks.
- Gate (verbatim): "corpus classes/exceptions slices green under tier-0."
- The only subprocess-visible seam the gate can use is `ap-conform` as
  documented by leaf 00.04 (`plans/antipythany/00-foundation-contract/
  orchestrator.md`): `ap-conform run --corpus tests/corpus/*.py --impl <cmd>`,
  comparing the three channels of S-06-1/C-27 (stdout bytes, stderr bytes,
  exit code) with separate-stream capture (S-06-2) and a scrubbed environment
  (S-06-15), plus `--tier 0` per the 02.03 gate text.
- Resolution order used by the suite for the harness: env `AP_CONFORM`
  (whitespace-split), then `ap-conform` on PATH, then `target/{debug,release}/`
  under the repo root. The impl command: env `APX_IMPL`, then `antipythany run`
  on PATH. Reference probe: env `AP_CONFORM_REFERENCE_PYTHON`, defaulting to
  `/opt/homebrew/bin/python3.14`.
- Because the harness contract is already fixed by 00.04 + spec 06, this leaf
  proposes NO new CLI (contrast T-01 G-1); it consumes the documented one.

## Corpus layout

```
tests/corpus/classes/    33 scripts c001..c033 + expected/<stem>.{out,err,rc}
tests/corpus/exceptions/ 15 scripts e001..e015 + expected/<stem>.{out,err,rc}
```

Expected files were produced by REAL runs of each script through
`/opt/homebrew/bin/python3.14` (3.14.7), recorded byte-exactly, and each case
was run twice to prove determinism. Every case catches the errors it pins
in-script and prints only `Type: message` (single-line form), so rc is 0 and
stderr empty for all 48 — the traceback path-text normalization problem of
spec 02's convention never enters this slice. A `test_expected_streams_are_clean`
check pins that property, and `test_no_nondeterministic_addresses_leak` greps
the expected bytes for `0x…` tokens (S-06-17 #4).

## Case table — filename → clauses → what is pinned

### classes/ (S-03-01..31)

| File | Cites | Pins (observed 3.14.7) |
| --- | --- | --- |
| c001_class_basics | 01, 04 | `(<class 'object'>,) True`; name/qualname/module; `type(A) is type`; mro tail; body-only dict keys |
| c002_class_dict_and_body | 03, 04 | decorator order `['d2', 'd1']`; method body misses class scope → `NameError: name 'x' is not defined`; `TypeError: 'mappingproxy' object does not support item assignment`; `__qualname__` not a dict key |
| c003_new_init_order | 05, 06 | anchor A2: `new\nnot an A str`, `__init__` never runs on non-instance |
| c004_new_returns_subclass_instance | 06 | subclass instance from `__new__` still gets `__init__` (`new\nnew\ninit B\ninit B`) |
| c005_newinit_arity | 07 | `B() takes no arguments`; `A7.__new__() takes 1 positional argument but 2 were given`; missing-arg binding error from `__init__` |
| c006_init_return_nonnone | 08 | `__init__() should return None, not 'int'` / `not 'R8b'` |
| c007_call_non_type | 05 | runtime-form `TypeError: 'int' object is not callable` (SyntaxWarning is compile-time-only, see masking M-3), `'C' object is not callable`, `__call__` works |
| c008_data_descriptor_priority | 09, 20, 21 | anchor A3 + A4: property beats instance dict (`prop`); `type(C.__dict__["x"])` is `property`; non-data shadowed (`99`) |
| c009_nondata_shadowed | 09, 10 | `nd`→`99 True`→`nd False`: instance assignment shadows, dict delete unshadows |
| c010_getattr_fallback | 09 | `gen:missing2 1 True`; hit in dict bypasses `__getattr__`; miss message `'D' object has no attribute 'nope'` |
| c011_property_setter | 10, 21 | setter routes (`10 get`); `property 'x' of 'RO' object has no setter`; `RO.x = 5` succeeds on the class |
| c012_delete_rules | 11, 20 | `dd-delete` before dict; `__delattr__` override (`delattr zzz`); `property 'x' of 'RO' object has no deleter`; data-descriptor `__set__` writes elsewhere |
| c013_del_inherited | 11 | inherited attr: `'C' object has no attribute 'x'` (instance's class name); after instance del the class value shows again |
| c014_class_lookup_metaclass | 12 | anchor A5: `C.mattr` → `7`, metaclass `__getattr__` → `M-got:whatever`, `C().mattr` AttributeError |
| c015_type_slots_priority | 12 | body `__bases__ = (int,)` does not change `BP.__bases__` (`(<class 'object'>,)`) |
| c016_slots_messages | 13 | anchor A6: full slots-set message + `False 1` + read-miss message + `member_descriptor type` |
| c017_slots_creation_errors | 13 | `__slots__ items must be strings, not 'int'`; `'x' in __slots__ conflicts with class variable`; bare-string slots = ONE slot named `xy` (gap GAP-6) |
| c018_slots_dict_member | 14 | `("x", "__dict__")` re-enables dict; dictless subclass of dict-member parent: slot desc + vars both work |
| c019_mro_examples | 15 | E1/E2/E3 exactly as the spec table + `['Redundant','B2','C2','A2','object']` accepted (S-03-16 redundant ordering) |
| c020_mro_errors | 15, 16 | `duplicate base class DA` (before MRO); `Cannot create a consistent method resolution order (MRO) for bases IA, IB` (anchor A9); `mro(object)` = `['object']` |
| c021_super_validation | 17 | `super(type, obj): obj (instance of int) is not an instance or subtype of type (SA).`; SA-obj-into-SB form; bare `super()` in a cell-less function → `RuntimeError: super(): no arguments` |
| c022_super_diamond_chase | 18, 30 | anchor A8: `D B C A` with MRO printed; inherited-`E(D)` gets the same chase bound to `E` |
| c023_super_classmethod | 19 | spec's exact shape: `B->B` (parent returns `cls.__name__`); `B->C` through an inherited hop |
| c024_descriptor_kinds | 20, 21 | classmethod same `cls` both paths; staticmethod identity `True` + `type` is `staticmethod`; bound method `c.f == c.f` True / `c.f is c.f` False |
| c025_dunder_bypass_instance_dict | 22, 23 | anchor A10 (`3`); instance-dict `__str__`/`__repr__` ignored by `str(a)`; explicit `a.__str__()` follows normal rules (`inst-str inst-repr`); default repr MASKED to shape predicates (M-1) |
| c026_special_missing_fallbacks | 23, 24 | `list(N())` → `[0, 1, 2]`; `object of type 'A' has no len()`; `'A' object is not iterable`; `__str__ returned non-string (type int)`; str-only class keeps default repr (masked predicate) |
| c027_binop_dispatch | 25 | anchor A11 (`R.radd L.add`); both-NotImplemented `unsupported operand type(s) for +: 'N25' and 'M25'`; `'<' not supported between instances of 'int' and 'A25'`; `1 + "x"` builtin message |
| c028_eq_negation_identity | 25 | `1 == REq()` → `reflected-eq`; `!=` is the negation of the __eq__ that ran; proper-subclass `__eq__` wins from the right; identity default |
| c029_augmented_rebind | 26 | `__iadd__` first; absent-iadd falls back to `__add__` + rebinds; NotImplemented-iadd fallback (`via-add`); list `+=` stays in-place (`True`) |
| c030_unary_contains_metaclass_hooks | 26, 27, 28 | `-u +u ~u abs(u)`; `in` via `__contains__` then iteration; pipeline `prepare → new → set_name → init` then `call …` wrapping instantiation (order pinned = observed, GAP-2) |
| c031_init_subclass_and_meta_call | 28, 29 | `Sub.tag == 7` + `__subclasses__()`; default hook rejects kwargs: `Bad.__init_subclass__() takes no keyword arguments`; metaclass `__call__` trace `call Widget (3,) {}`; forwarding-metaclass requirement (GAP-3) |
| c032_metaclass_resolution | 02 | most-derived wins (`MB`); unrelated → `metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass of the metaclasses of all its bases` (GAP-1); zero bases verbatim → `NotAMeta() takes no arguments` (spec-quoted) |
| c033_isinstance_issubclass | 31 | hierarchy truths; self/object subclass; `object.__bases__ == ()`; tuple + `U1 | U2` union args; `isinstance() arg 2 must be a type, a tuple of types, or a union`; `issubclass() arg 1 must be a class`; `__instancecheck__`/`__subclasscheck__` hooks (`isinstance(42, Weird)` True) |

### exceptions/ (S-03-32/33/34)

| File | Cites | Pins (observed 3.14.7) |
| --- | --- | --- |
| e001_raise_requires_baseexception | 01, 30, 32, 33 | bare `raise SubError` instantiates; `raise 5` / `raise "boom"` / `raise NotExc()` → `exceptions must derive from BaseException`; `ValueError("a", 2)` triple `('a', 2) / ValueError('a', 2) / ('a', 2)` |
| e002_hierarchy_structure | 32 | `BaseException.__bases__ == (<class 'object'>,)`; the two sorted subclass lists exactly as the spec; `ValueError`/`ZeroDivisionError` MRO chains; EG caught by `Exception`, BEG only by `BaseException`; the spec's `(True True False False False)` tuple |
| e003_except_target_validation | 34 | anchor A12: `catching classes that do not inherit from BaseException is not allowed` — clause NOT skipped; tuple any-element; later-clause fallthrough; tuple-with-int also raises |
| e004_nested_tuple_rejected | 32, 34 | **nested tuple targets are REJECTED** on 3.14.7 (contradicts S-03-34 "nested tuples allowed" — GAP-5); non-class element in a tuple rejected too |
| e005_custom_hierarchy | 32, 33 | 3-level user chain MRO; catch via grandparent; `__init__` calling `super().__init__(f"E{code}")` → `WithInit 42 E42 ('E42',)` |
| e006_except_is_isinstance | 31, 32 | except-matching IS the isinstance/hierarchy rule: `except LookupError` catches KeyError; bare `except BaseException` catches SystemExit; first-clause-wins (S-02-30 cite); `UnboundLocalError ⊂ NameError`; GeneratorExit not ⊂ Exception |
| e007_raise_bare_class | 33 | bare class → `MyErr()` empty args; instance identity preserved at handler (`e is inst` True); `__init__` runs at the raise site |
| e008_exceptiongroup_structure | 32, 33 | EG/BEG membership + MRO; `.message`/`.exceptions`/`.args`; nesting; BEG not catchable by `except ValueError` for a member that would match |
| e009_warning_subtree | 32 | `Warning ⊂ Exception`; sorted Warning subtree (3.14.7 observed list); `SyntaxWarning` raised/catchable as an ordinary exception |
| e010_builtin_exc_class_shape | 32, 33 | builtins have S-03-04 class shape; `type(ValueError) is type`; zero/one/two-arg str/repr/args |
| e011_except_target_nonclass | 34 | `except 5`, `except object`, `except "ValueError"` — all catch-time TypeError, message pinned |
| e012_bare_class_init_arity | 33 | bare raise of a needs-args exception fails with the BINDING error (`NeedsArgs() missing 1 required positional argument: 'code'`), not the derive-from message |
| e013_exception_new_init | 33, 06 | exceptions are normal classes: user `__new__`/`__init__` run at the raise site (`new` then `init`), substitution rules apply |
| e014_full_tree_catchable | 32, 34 | live-derived `Exception.__subclasses__()` (len 22, names printed) — every member raisable + catchable through a tuple of the whole set |
| e015_exception_class_shape | 04, 32, 33 | user exception qualname/module/bases; `e.__class__ is type(e)`; KeyError one-arg str is `repr`-quoted (`KeyError 'k'`) — cross-cite spec 02 S-02-29 |

## Masking decisions (assertion-strengthening rewrites, never weakening)

- **M-1 (default object repr, S-06-17 #4 / C-24):** `<__main__.A object at 0x…>`
  is address-bearing and corpus-prohibited as a byte anchor. c025/c026 therefore
  pin the *shape* via boolean predicates printed to stdout
  (`repr(a).startswith("<__main__.A object at 0x")`, `.endswith(">")`,
  `str(a) == repr(a)`) plus the instance-level `__str__`/`__repr__` *explicit*
  call results. The clause assertions (bypass vs explicit) are fully preserved.
- **M-2 (`c029` `iadd <Other object at 0x…>`):** the original draft printed the
  operand object into the trace line, leaking an address. Rewritten so `__iadd__`
  prints a constant (`iadd`); the rebind and fallback behavior remain pinned.
- **M-3 (SyntaxWarning on literal `5()`):** S-03-05 pairs the message with a
  *compile-time* SyntaxWarning that fires only for the literal call form and
  writes to stderr at compile. ap-conform's scrubbed env makes warning capture
  implementation-side noise unrelated to the class clause; c007 pins the runtime
  callable-protocol message via a bound variable (no warning, rc 0) and notes
  the exclusion. The literal-form SyntaxWarning belongs to spec 02's
  warning-format territory, not 02.03's.
- **M-4 (set/var order):** `sorted()` everywhere `__subclasses__()`/`vars()`
  are printed (C-26); dict `vars()` entries are pair-sorted.
- **M-5 (`ExceptionGroup` construction arg:** `.args` of EG prints
  `[ValueError('x'), ...]` — exception *reprs* inside a list are deterministic
  (class + message only, no addresses), kept unmasked.

## CONTRACT-GAPS (findings to file back per README §Protocol 5)

- **GAP-1 — S-03-02's stated failure mode is the wrong message.** The clause
  says "if `M` is not a subclass of a base's metaclass, class creation fails
  with `TypeError`" and the spec's observed line covers only the zero-base
  `NotAMeta` case. With bases, 3.14.7 raises a *different, spec-unquoted*
  message: `metaclass conflict: the metaclass of a derived class must be a
  (non-strict) subclass of the metaclasses of all its bases` (c032 pins it).
  The spec should quote it. Not a contradiction of normative force, but the
  byte-exact gate needs the string in the spec pack.
- **GAP-2 — S-03-27 hook ordering is wrong vs 3.14.7.** Clause: "`M.__new__` →
  class object; `M.__init__(cls, …)`; **then** `__set_name__` hooks … and
  `B.__init_subclass__` run." Observed: `__set_name__` fires INSIDE
  `type.__new__` (before `M.__new__` returns), and `__init_subclass__` fires
  before `M.__init__` completes — trace: `prepare → new-start → set_name →
  new-end → init_subclass → …`. The spec's "then" ordering would produce a
  different stdout trace. c030/c031 pin the OBSERVED order (reference wins by
  C-02) and the clause text needs a PR.
- **GAP-3 — S-03-29's "called … with the class-statement keyword arguments"
  is conditional in practice.** The hook only receives the kwargs when they
  reach `type.__new__` (plain `type` metaclass, or a custom metaclass that
  forwards `**kw` into `super().__new__`). A custom metaclass that drops them
  leaves the hook with none (probe: `Meta.__new__()` got an unexpected keyword
  argument` when un-forwarded vs missing-positional when swallowed).
  c031 pins the two supported paths and documents the condition.
- **GAP-4 — (resolved during authoring; retained as a caveat).** S-03-19's
  worked example (`B.m()` → `B->B` when the parent returns `cls.__name__`) is
  consistent on 3.14.7 — an early probe misread it because the probe classes
  were differently named. c023 pins the spec's exact shape (`B->B`, plus
  `B->C` through an inherited hop). No spec change needed; recorded so the
  corpus's citation trail shows the example was verified, not assumed.
- **GAP-5 — S-03-34 "nested tuples allowed" is FALSE on 3.14.7.**
  `except ((KeyError, ValueError),):` and every nested-tuple variant raise
  `TypeError: catching classes that do not inherit from BaseException is not
  allowed` at catch time (e004). Direct contradiction; needs a spec PR. Until
  corrected, C-02 makes the reference bytes normative and the corpus pins
  rejection.
- **GAP-6 — bare-string `__slots__` is unpinned by S-03-13.** "non-empty
  iterable of strings" reads like `__slots__ = "xy"` → slots `x` and `y`
  (iterating a string), but 3.14.7 creates ONE slot named `xy`. e/c017 pins the
  observed single-slot behavior; spec should state the string-is-one-name rule.
- **GAP-7 — reference-build drift (all leaves' shared caveat).** Expected bytes
  recorded on `/opt/homebrew/bin/python3.14` = **3.14.7**; C-01 pins **3.14.0**
  and marks non-pinned-patch bytes provisional (already an OPEN item in
  00-foundation PROGRESS.md). Re-verification is one `ap-conform`
  self-comparison run when a 3.14.0 build lands.

## Per-test citations (suite `tests/t06_classes/test_t06_classes.py`)

| Test | Assertion | Cites |
| --- | --- | --- |
| `test_header_cites_existing_clauses` ×48 | line 1 is `# Cites: S-03-…` and every id exists in spec 03 | tests/README.md corpus rules |
| `test_every_spec03_must_clause_is_covered` | union of header cites ⊇ all 34 S-03 ids | 02.03 Expose completeness; C-04 |
| `test_slices_meet_quotas` | classes ≥ 30, exceptions ≥ 15 cases (floor only; corpus grows, C-33) | task contract |
| `test_reference_reproduces_expected_bytes` ×48 | ref stdout==`.out`, stderr==`.err`, rc==`.rc`, byte-exact | S-06-1/2; C-24/C-27; C-01 caveat GAP-7 |
| `test_expected_streams_are_clean` ×48 | rc 0, stderr empty, stdout ends with `\\n` for every case | spec-03 anchor section rule ("Expected output is byte-exact stdout … rc 0"); S-06-14 |
| `test_no_nondeterministic_addresses_leak` ×48 | no `0x[0-9a-f]{4,}` in expected bytes | S-06-17 #4; C-24 |
| `test_ap_conform_slice_green_tier0` ×2 (XFAIL) | `ap-conform run --corpus <slice>/*.py --impl … --tier 0` exits 0 | 02.03 Gate verbatim; 00.04 CLI; C-31 tier-0 |
| `test_ap_conform_case_green_tier0` ×48 (XFAIL) | per-case ap-conform exit 0 | S-06-1 triad; C-04; README gate semantics |

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t06_classes -q
```

From the repo root. Today: **194 passed, 50 xfailed, 0 errors.** Overrides:
`AP_CONFORM=<cmdline>` (harness), `APX_IMPL=<cmdline>` (implementation),
`AP_CONFORM_REFERENCE_PYTHON=<path>` (reference).

## xfail policy

- The 50 ap-conform cases are `xfail(strict=False)` (T-02 exemplar parity):
  the seam (00.04 harness exists as a crate but the `antipythany` impl command
  and ap-obj semantics do not), so bodies raise `FileNotFoundError` inside the
  XFAIL envelope.
- After 02.03 lands: cases run for real; XPASS is expected and reported;
  failures are gate failures for 02.03. Fix the implementation, never the test
  (master.md execution protocol; C-33). A failing case that traces back to
  GAP-1..GAP-6 is resolved by a PR against `docs/spec/03-classes.md`, never by
  silently picking semantics in the impl.
- The 194 fixture re-verification cases are NOT xfail — they must stay green;
  a break there means the recorded reference bytes drifted (C-01 probe change),
  which is a harness/version finding, not a gate finding.

## Protocol notes

- Tests commit BEFORE 02.03 completes (README §Protocol 3); gate-time runner is
  an agent that wrote neither these tests nor ap-obj (§Protocol 4).
- Spec contradictions found: GAP-2 and GAP-5 are hard contradictions (clause
  text vs measured reference behavior on 3.14.7) and need PRs against
  `docs/spec/03-classes.md`; GAP-1, GAP-3, GAP-6 are under-specifications;
  GAP-4 was resolved during authoring (spec example verified consistent);
  GAP-7 is the shared patch-pin caveat. Corpus headers on c017/c030/c031/c032/
  e004 cite their gap ids inline so the findings travel with the bytes.
