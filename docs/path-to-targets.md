# ANTIPYTHANY - Path to Target Projects (working doc, updated as plans evolve)

Goal state per master.md tier table: run scrapy (T1), pytest (T2),
hermes-agent (T3), fastapi (T4), comfyui (T5) as TEST DATA - internal
validation only, never published.

## Current state (updated 2026-10-17)

Working: parse -> IR -> verify -> tier-0 EXECUTION. Corpus: hello 1/1,
types 1/1, fstrings 33/35, values 43/47, stdlib 0/48. Rust 146/0, pytest
1223+/0. Generators, closures, exceptions, tracebacks, 37 builtins all
byte-exact vs 3.14.7.

In flight: 02.03 (classes/MRO/protocols/real exceptions - unblocks
V048/V050/V066/f12) + 02.08-lite (native modules: math/json/hashlib/
itertools/random/etc + native-import registry - raises stdlib 0/48).

## Dependency analysis: what each target needs beyond current state

Corpus stdlib slice imports (52 modules incl. transitive): abc argparse ast
atexit bisect builtins codecs collections contextlib copy dataclasses
encodings enum errno fnmatch functools genericpath glob heapq importlib
inspect io itertools json keyword linecache math numbers operator os pathlib
platform posixpath pprint re reprlib stat string struct sys tempfile
textwrap threading time token tokenize traceback types typing warnings
weakref.

KEY INSIGHT: pathlib/dataclasses/inspect/unittest are PURE-PYTHON files in
CPython. Two strategies:
  A) VENDOR CPython's pure-python stdlib files (PSF license, attribution in
     notices - compatible) and run them on our runtime once class machinery
     is solid. This is what gets us pathlib/dataclasses/typing/asyncio
     cheaply. They stress-test the interpreter better than native rewrites.
  B) NATIVE Rust rewrites (like the 02.08-lite modules) where performance or
     OS access matters (os, sys, time, io, socket).
Correct mix: B for machine-facing modules, A for logic-heavy pure modules.

## Remaining work to T1 (scrapy, pure path, no lxml)

1. 02.03 classes (IN FLIGHT) - blocking everything OOP: scrapy spiders are
   classes, twisted deferreds are classes.
2. 02.08 stdlib: ossubset (environ, paths, getcwd), io (StringIO/BytesIO),
   time, selectors (T1 uses selectors.DefaultSelector), contextlib
   (AbstractContextManager, closing, contextmanager decorator - pure),
   weakref (pure-ish), logging (pure, big), queue (pure + threading prims).
3. import-of-file machinery: sys.path, importlib basics, packages
   (__init__.py), namespace resolution - THE missing piece for running any
   third-party pure-Python package (twisted, scrapy).
4. Plan 04: uv subprocess to fetch pure-python wheels into a build cache +
   package them into the compile input set. (Pure wheels = zip of .py
   files: unpack, add to import path, compile.)
5. Threading (02.07) - twisted's reactor is single-threaded; defer to post-T1.
6. T1 gate: `scrapy crawl` on a file:// spider with an item pipeline,
   byte-compare items.jsonl vs CPython-run reference.

## After T1

- T2 pytest: assertion rewriting (AST), inspect/frames/code objects,
  traceback formatting depth. Needs 01.04-style introspection APIs + import
  machinery + class system. No new native modules.
- T3/T4 hermes-agent/fastapi: pydantic-core (Rust extension) = Plan 10, OR
  pydantic v1 pure-python fallback (slower but pure). Decide at 10.01.
- T5 comfyui: torch stack = full Plan 10.

## Sequenced plan (post-02.03/02.08-lite)

1. 02.03 classes (in flight)
2. 02.08-lite native modules + import registry (in flight)
3. 02.04a: import-of-file machinery (sys.path, packages, importlib.open)
   <- NEW LEAF, not in original tree; insert into Plan 02
4. 02.04 GC (MMTk swap) - can parallel with 3
5. Vendor pure-python stdlib (pathlib/dataclasses/typing/contextlib/...)
   <- NEW LEAF
6. 02.05 tier-1 (SPLIT per pon risks) - after correctness complete
7. Plan 04 packaging (uv fetch pure wheels)
8. T1 scrapy attempt
