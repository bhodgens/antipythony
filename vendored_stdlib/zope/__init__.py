"""antipythony deviation shim — zope (namespace package placeholder).

vendored_stdlib precedes site-packages on sys.path, so this package SHADOWS
the real zope.interface for the runtime. See docs/lxml-shim.md
("zope.interface stub") for surface and rationale: the real zope.interface's
ro.py `_ClassBoolFromEnv.__get__` identity-scan (`cls.__dict__[k] is self`
across `dir(cls)` / `__mro__`) does not resolve under the v1 FnRegistry;
interfaces are advisory for the T1 gate (scrapy/parsel import + selector
paths).
"""
