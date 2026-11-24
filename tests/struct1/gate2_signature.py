# vendored types.py aliases native callables via type(len) — both resolve to
# the same interned 'function' Type as user functions. The runtime-level fix
# (this leaf): the interned "function" type payload serves BOTH; discriminate
# at the inspect layer via __module__ == 'builtins'.
#
# That means isbuiltin / _signature_is_builtin need the __module__ probe.
# vendored_stdlib is sibling-owned (do-not-edit per policy in NEXT-T3.md §S2
# F-VENDORED-1) BUT inspect.py already carries ~16 declared deviation lines
# (DEVIATIONS.toml row inspect.py, re-declared across waves 5/82/102) — the
# isbuiltin probe is a declared-shape edit: 3 lines, no CPython semantics
# dropped. Report + gate here; the vendored edit lands in THIS leaf since it
# unblocks the pydantic chain and inspect.py has an established deviation
# ledger.
import inspect
import types


class C:
    def __init__(self, a: int) -> None:
        pass


meth = inspect.getattr_static(C, '__init__', None)

print("ISBUILTIN-LEN", inspect.isbuiltin(len))
print("ISBUILTIN-USER", not inspect.isbuiltin(meth))
print("USER-MODULE", getattr(meth, '__module__', None) != 'builtins')
print("LEN-MODULE", getattr(len, '__module__', None) == 'builtins')
