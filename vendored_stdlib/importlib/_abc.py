"""Stub of importlib._abc (CPython 3.14 has this as a separate module).

The vendored importlib.abc does `from ._abc import Loader`; the CPython file
defines Loader as an ABC (metaclass=abc.ABCMeta). Our v0 runtime: a plain
class stub — the surfaces importlib.abc/util need are presence + subclassing.
"""

import abc as _abc_mod


class Loader(metaclass=_abc_mod.ABCMeta):
    """Stub ABC for importlib.loaders (CPython: abc.ABC-based)."""

    def create_module(self, spec):
        return None

    def exec_module(self, module):
        pass


class _LazyLoader:
    pass
