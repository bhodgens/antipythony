# P.main: imported lazily via P.__getattr__ WHILE P is mid-__getattr__.
# The from-import of P.helper must consult P's module dict + __getattr__
# (sys.modules-resolved partial module) — NOT re-enter the importer.
from collections.abc import Callable

from P import helper

OUTPUT = "FROM-SUB:" + helper
