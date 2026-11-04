# lxml compatibility shim (leaf 02.08-c)

Status: **partially complete — core shim landed and gated; gate probe still red.**

The antipythony runtime cannot use real lxml (C extension; APX-CEXT policy
forbids native wheels). This shim provides a pure-Rust `lxml`-compatible
surface in `crates/ap-run/src/lxml.rs` with no libxml2 and no new Cargo
dependencies.

## Architecture

- `crates/ap-run/src/lxml.rs` (~2.9k lines): `LxNode` tree, Rust-side HTML/XML
  tokenizer (no html.parser dependency for the native path), XPath subset
  evaluator, serializer (XML + HTML modes with void-element and
  auto-close rules), etree/html builders.
- Registration: `builtins_modules.rs` native-module registry —
  `lxml`, `lxml.etree`, `lxml.html`; `from lxml import etree` works via
  submodule objects linked into the lxml module dict.
- Element attribute reads (`tag`, `text`, `tail`, `attrib`, `xpath`, …) via
  `native_attr`; bound methods via `__ap_lxml__.<name>` markers in
  `call_native_marker`; `len()`/`[]`/iteration wired through `special_lookup`.

## Surface covered

`lxml.etree`: `Element`, `SubElement`, `Comment`, `fromstring`, `XML`, `tostring`
(`encoding='unicode'`, `with_tail`, `method='html'`), `XMLParser`,
`HTMLParser`, `XMLSyntaxError`, `XPath` (subset), `ErrorTypes` (int()-able
flags), `iselement`, `cleanup_namespaces`, `tounicode`.

`lxml.html`: `fromstring`, `document_fromstring`, `HtmlElement` with
`text_content`, `itertext`, `body`/`head`/`links`/`classes`,
`make_links_absolute`.

`_Element` protocol: `tag`, `attrib`, `text`, `tail`, `get`, `set`, `find`,
`findall`, `iter`, `xpath` subset (see below), `len()`, indexing (incl.
negative), iteration, `__setitem__` inert stores.

### XPath subset

`//tag`, `/tag`, `./tag`, `tag`, `*`, `@attr`, `@*`, `text()`,
`contains(@class,'x')`, `[pred]` (positional + attribute predicates),
`descendant-or-self::tag`, `child::tag`, attribute-value results
(`_ElementUnicodeResult`-like strings), node-set results. `//tag` is
`descendant-or-self::node()/child::tag` — the context node itself can match.

## Known gaps / remaining blockers

1. **GATE BLOCKER (not lxml):** runtime module-exec poisoning — a
   `NameError: name 'parse' is not defined` raised from the *runtime's own
   print/file internals* when `parsel.selector` is exec'd via the shim
   re-exec path. Same failure class as the earlier calendar
   `setfirstweekday` poisoning (FnRegistry keyed by bare function name). A
   fix to the runtime's global-function registry will likely clear both.
2. PEP 604 unions of native-class markers (`etree.XMLParser |
   etree.HTMLParser`) now build `_Union` (interp.rs); unions of *arbitrary*
   user functions still TypeError — acceptable for annotations-only use.
3. XPath: no `position()`, `last()`, arithmetic in predicates, no
   `following-sibling`/`preceding` axes beyond the subset above.
4. No `iterparse`, no `XSLT`, no DTD/validation, no custom resolvers.
5. Serializer: text/tail escaping matches real lxml for the differential
   corpus; exotic encodings untested.

## Differential tests

`crates/ap-run/tests/lxml_differential.rs`: 24 cases, byte-compared against
CPython 3.14 + real lxml 6.1.3 (reference venv `/tmp/lxml-ref-venv`; set
`AP_RUN` env var to point at a non-default runtime binary). Tests that
require no scrapy chain are green; cases that traverse
`html.fromstring`→document semantics pass. Skipped-if-reference-missing
semantics keep CI green on machines without the venv.

## Deviations from verbatim vendored stdlib

- `vendored_stdlib/_markupbase.py`: added (verbatim CPython 3.14; was
  missing, needed by html.parser).
- `vendored_stdlib/importlib/_abc.py`: `class Loader(metaclass=...)` —
  breaks an abc↔importlib metaclass cycle in the runtime (DEVIATIONS entry
  required).
- `vendored_stdlib/calendar.py`: property-based `Calendar.firstweekday`
  rewritten as plain attribute + module-level functions (runtime FnRegistry
  poisoning workaround; revisit when registry is fixed).
- `vendored_stdlib/zope/`: new advisory-only stub package (Interface,
  implementer, provider, alsoProvides, classAlsoProvides, AdapterRegistry,
  ObjectSpecificationHierarchy) shadowing site-packages zope.interface for
  the runtime only; real zope ro.py crashes on `klass.__dict__` scans.
- `vendored_stdlib/parsel/`: shim package that re-execs the real
  site-packages parsel (vendored dir precedes site-packages on sys.path);
  provides `translation` callable used by cssselect.

## License notes

No new crates added; no Cargo dependency changes. Everything is
workspace-local Apache-2.0 WITH LLVM-exception. The zope/parsel shims are
original code (no upstream copying).
