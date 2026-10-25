# T1 GATE: GREEN (2026-10-24)

## Result

`scrapy 2.19.0` imports and runs under the antipythany runtime. The file://
spider crawl produces items.jsonl byte-identical to CPython 3.14.0
(4 items: ALPHA WIDGET / BETA GADGET / GAMMA GIZMO + SUMMARY count=3).

Probe: `SCRAPY IMPORT OK 2.19.0` (exit 0).
Compare: `diff /tmp/t1/items_cpy.jsonl <repo>/items.jsonl` -> identical.

## What it took (chain of ~20 blockers cleared across 7 dispatches)

1. live-sys.path importer fix (SYS_PATH_VIEW) - commit a14fe0d
2. lxml native shim: etree/html/xpath-subset (~2.9k lines, no libxml2) - a021802
   plus forced interpreter fixes: FnRegistry module-exec poisoning,
   PEP 604 callable unions, metaclass bases arity, os/decimal/unicodedata
3. zope stub (Interface/AdapterRegistry/Declaration iterable/implementedBy)
   + parsel re-exec shim + importlib/_abc cycle break - a021802, 4b228b8
4. vendored stdlib wave: pickle, copyreg, hmac, gzip, asyncio, concurrent,
   socket constants, zlib/compression stubs, encodings pkg (120+ codecs),
   datetime, decimal, logging pkg, email, urllib, xml - a14fe0d, 4b228b8
5. final wave (deleg_928af643): attr_get __spec__/__doc__ shadowing order,
   singledispatch register-by-annotation + dispatch, namedtuple MRO fields,
   dict view objects, dict.__setitem__, property.setter on subclasses,
   bytes.fromhex, os.pipe/utime/realpath-Path, select.select/poll/kqueue shim,
   ssl stub, importlib.metadata, packaging, sys platlibdir/abiflags,
   Path.read_text - uncommitted, recovery leaf fixing enum regression

## Known gaps (non-blocking for T1)

- enum import regression in the last uncommitted wave - recovery leaf
  (deleg_36e8ddee) fixing; s024 corpus case red until then
- hashlib hexdigest/digest attr-callability (stdlib_modules differential 1 case)
- asyncio internals partially shimmed (kqueue select shim, _ConstantsContainer)
- zlib/compression/brotli: import-level stubs; real compression NotImplementedError
- C extensions remain APX-CEXT-refused (lxml/bytes-real-lxml/crypto backends)

## Ecosystem tier status

- T1 scrapy: GATE GREEN (this doc)
- T2 pytest: next (builds on unittest + traceback surfaces already landed)


## Tier status update (2026-10-25)

- T1 scrapy: DONE (this doc).
- T2 pytest: import gate GREEN (pytest 9.1.1, all 48 _pytest submodules;
  pluggy imports). pytest.main reached config/plugin loading. Remaining
  blocker: plugin-spec resolution + int(ExitCode) (see issues #7, #10 and
  the PEP-342 lazy-generator feature, landed in 7c0a645).
- PEP-342 lazy generators landed (7c0a645): real send/throw/close/
  StopIteration(value) via corosensei stackful fibers. Unblocks pytest.main,
  s018, and future async.
