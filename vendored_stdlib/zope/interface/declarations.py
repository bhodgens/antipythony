"""antipythany deviation shim — zope.interface submodules used by
twisted.python.components (the scrapy reactor wiring). v0 inert surfaces.

See _shim.py, docs/lxml-shim.md.
"""


class InterfaceClass:
    def __init__(self, name=None, bases=(), attrs=None, __doc__=None,
                 __module__=None):
        self.__name__ = name
        self.__doc__ = __doc__

    def __call__(self, obj, alternate=None):
        return False

    def providedBy(self, obj):
        return False

    def implementedBy(self, cls):
        return False

    def isOrExtends(self, other):
        return False


class Attribute:
    def __init__(self, name, type_name=None):
        self.__name__ = name


class Invalid(Exception):
    pass


# zope.interface.declarations — advisory stubs used by
# twisted.python.components.
def provider(iface):
    """Wave-105: interface-conditional decorator (real zope API).

    Returns a decorator that declares `iface` as provided by the decorated
    object if it doesn't already. v0: records nothing (inert), matching the
    rest of this shim's surface.
    """
    def _decorator(ob):
        return ob
    return _decorator


def implementer(*ifaces):
    def _deco(cls):
        return cls

    return _deco


def implementedBy(cls):
    return False


def classImplements(cls, *ifaces):
    return None


def classImplementsOnly(cls, *ifaces):
    return None


def implementer_only(*ifaces):
    def _deco(cls):
        return cls

    return _deco


def providedBy(obj):
    return _EmptySpec()


def alsoProvides(obj, *ifaces):
    return None


def directlyProvides(obj, *ifaces):
    return None


def classProvides(*ifaces):
    def _deco(cls):
        return cls

    return _deco


def noLongerProvides(obj, iface):
    return None


def Declaration(*ifaces):
    return _EmptySpec()


class _EmptySpec:
    def __contains__(self, iface):
        return False

    def __iter__(self):
        return iter(())

    def extends(self, iface, strict=True):
        return False

    def isOrExtends(self, iface):
        return False

    def providedBy(self, obj):
        return False

    def __call__(self, obj):
        return False


# zope.interface.interface module surface.
class _InterfaceModule:
    InterfaceClass = InterfaceClass
    Attribute = Attribute

    adapter_hooks = []


interface = _InterfaceModule()
