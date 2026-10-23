"""antipythany deviation shim — zope.interface implementation (see docs/lxml-shim.md).

Minimal advisory-only surface for the T1 chain (twisted.python.components,
scrapy.interfaces deprecation shim, itemadapter): Interface tokens,
implementer() no-op decorators, providedBy(), registration helpers, and inert
registries. Real interface verification is a documented Plan-02 gap.
"""

__all__ = [
    'Interface', 'Attribute', 'Invalid', 'implementer', 'implementation',
    'providedBy', 'alsoProvides', 'directlyProvides', 'noLongerProvides',
    'classImplements', 'classProvides', 'verifyObject', 'verifyClass',
    'Declaration', 'invariant', 'namedAny', 'provider',
]


class InterfaceClass:
    """Minimal Interface token."""

    def __init__(self, name=None, bases=(), attrs=None, __doc__=None,
                 __module__=None):
        self.__name = name or 'Interface'
        self.__bases__ = tuple(bases)

    def __call__(self, obj, alternate=None):
        return alternate

    def providedBy(self, obj):
        return self

    def isOrExtends(self, other):
        return True

    def __contains__(self, other):
        return True

    def __repr__(self):
        return '<Interface %s>' % self.__name


Interface = InterfaceClass('Interface')


class Attribute:
    def __init__(self, name, type_name=None):
        self.__name__ = name
        self.type_name = type_name


class Invalid(Exception):
    pass


class invariant:
    def __init__(self, func):
        self.func = func


def implementer(*ifaces):
    def _deco(cls):
        return cls
    return _deco


implementation = implementer
provider = implementer


def providedBy(obj):
    return Interface


def alsoProvides(*args, **kwargs):
    return None


def directlyProvides(*args, **kwargs):
    return None


def noLongerProvides(*args, **kwargs):
    return None


def classImplements(*args, **kwargs):
    return None


def classProvides(*args, **kwargs):
    return None


def verifyObject(iface, obj, tentative=False):
    return True


def verifyClass(iface, klass, tentative=False):
    return True


class Declaration:
    pass


def namedAny(name):
    parts = name.rsplit('.', 1)
    if len(parts) != 2:
        raise ValueError('bad name: %r' % (name,))
    mod = __import__(parts[0], fromlist=[parts[1]])
    return getattr(mod, parts[1])
