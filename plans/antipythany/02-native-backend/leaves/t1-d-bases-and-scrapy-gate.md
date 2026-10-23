# Leaf T1-d: clear `bases must be types, not 'function'` in `import scrapy`

## State (2026-10-23, post 9fe23b4)
`cd /tmp/t1/t1proj && ../../git/antipythany/target/debug/antipythany run --no-cache
/tmp/scrapy_gate.py` fails with `TypeError: bases must be types, not 'function'`.

## Known surface
- Check sites: classes.rs:139 (dynamic_type), classes.rs:686, builtins.rs:3911.
- base_type_ref (classes.rs:1144) already synthesizes for NamedTuple/TypedDict/
  classmethod/staticmethod/property/GenericAlias. The new failure is a plain
  FUNCTION base outside that allowlist.
- Probe: `from zope.interface import Interface` stub + `IFoo.implementedBy` is
  ALSO missing from the shim (vendored_stdlib/zope/interface/_shim.py) - twisted
  calls it; add `implementedBy`/`implementedBy` classmethod on InterfaceClass
  returning a truthy Declaration-ish token.

## Deliverables
1. Extend base_type_ref: any Function/BuiltinFn base that is NOT one of the
   known-descriptor names synthesizes an object-derived TypeObject named after
   the function (same pattern as NamedTuple case). CPython would TypeError, but
   our policy for the T1 chain is create-then-degrade; document in
   docs/lxml-shim.md DEVIATIONS.
2. Add zope shim: InterfaceClass.implementedBy (staticmethod, returns
   _implements token truthy), also ImplementedBy fallback module fn if used.
3. GATE: the scrapy probe above prints `SCRAPY IMPORT OK <version>`.
4. Then run the file:// spider crawl in /tmp/t1/t1proj and byte-compare
   items.jsonl vs CPython (docs/path-to-targets.md). Report either GREEN or the
   next single blocker with a minimal repro.
5. Gates to hold: cargo test --workspace -q (lxml_differential may stay red
   until the parallel fixer lands - leave that file alone), pytest tests/ -q
   1912 passed, corpus rates never decrease.

Rules: fix our runtime/shims, never fork scrapy/twisted; commit nothing.
