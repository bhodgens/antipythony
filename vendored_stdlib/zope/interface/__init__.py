"""antipythany deviation shim — zope.interface package (see _shim.py and
docs/lxml-shim.md). Re-exports the advisory-only surface."""
from ._shim import *  # noqa: F401,F403

# T1-d: twisted.python.components imports the submodules directly.
from . import declarations  # noqa: F401
from . import adapter  # noqa: F401
import importlib as _importlib

interface = _importlib.import_module("zope.interface.interface")
