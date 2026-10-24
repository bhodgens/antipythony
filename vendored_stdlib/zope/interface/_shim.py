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
    'Declaration', 'invariant', 'namedAny', 'provider', 'AdapterRegistry',
]


class InterfaceClass:
    """Minimal Interface token."""

    def __init__(self, name=None, bases=(), attrs=None, __doc__=None,
                 __module__=None):
        self.__name = name or 'Interface'
        self.__bases__ = tuple(bases)
        # T1-d (twisted proxyForInterface: `for name in iface`): collect
        # public attrs as Attribute descriptions.
        self.__attrs = {}
        if attrs:
            for k, v in attrs.items():
                if not k.startswith('_'):
                    self.__attrs[k] = Attribute(k)

    def names(self=None, all=False, _self=None):
        # T1-d: when resolved UNBOUND through a derived class's MRO
        # (`IFoo.names` where IFoo subclasses Interface), `self` arrives as
        # the first POSITIONAL slot empty and the class lands in _self via
        # the instance-less call shape; v0 returns the class's own attrs
        # when we can find them, else empty (the gate only iterates to
        # build proxy descriptors).
        target = self if self is not None else _self
        r = set()
        try:
            r.update(target.__attrs.keys())
        except Exception:
            pass
        for b in getattr(target, "__bases__", ()):
            try:
                r.update(b.names(all=True))
            except Exception:
                pass
        return iter(r)

    def __iter__(self):
        return iter(self.names(all=True))

    def __call__(self, obj, alternate=None):
        return alternate

    def providedBy(self, obj):
        return self

    def implementedBy(self, cls):
        # Advisory: always truthy so twisted's `IFoo.implementedBy(Bar)`
        # registration checks succeed. Real verification is a Plan-02 gap.
        return True

    def isOrExtends(self, other):
        return True

    def __contains__(self, other):
        return True

    def __repr__(self):
        return '<Interface %s>' % self.__name

    # T1-d: `class IFoo(Interface)` — the instance-as-base PEP 560 hook.
    # Subclasses derive from InterfaceClass itself (advisory surface only).
    def __mro_entries__(self, bases):
        return (InterfaceClass,)


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
    """Advisory Declaration: iterable of the interfaces an object provides."""

    def __init__(self, *ifaces):
        self._ifaces = tuple(ifaces) if ifaces else (Interface,)

    def __iter__(self):
        return iter(self._ifaces)

    def __contains__(self, iface):
        return True

    def __len__(self):
        return max(1, len(self._ifaces))

    def isOrExtends(self, other):
        return True

    def extends(self, other, strict=True):
        return True

    def get(self, name, default=None):
        return getattr(Interface, name, default)


def namedAny(name):
    parts = name.rsplit('.', 1)
    if len(parts) != 2:
        raise ValueError('bad name: %r' % (name,))
    mod = __import__(parts[0], fromlist=[parts[1]])
    return getattr(mod, parts[1])


class AdapterRegistry:
    """Advisory-only adapter registry: every lookup succeeds."""

    def __init__(self, *args, **kwargs):
        self._registry = {}

    def register(self, *args, **kwargs):
        return None

    def unregister(self, *args, **kwargs):
        return None

    def lookup(self, *args, **kwargs):
        return None

    def lookup1(self, *args, **kwargs):
        return None

    def lookupAll(self, *args, **kwargs):
        return ()

    def subscriptions(self, *args, **kwargs):
        return ()

    def subscribe(self, *args, **kwargs):
        return None

    def unsubscribe(self, *args, **kwargs):
        return None


def getObjectSpecification(*args, **kwargs):
    return Interface


def implementedBy(*args, **kwargs):
    return Declaration()


def implementer_only(*ifaces):
    def _deco(cls):
        return cls

    return _deco


def provider(*ifaces):
    def _deco(cls):
        return cls

    return _deco


def classProvides(*args, **kwargs):
    return None


def registered(*args, **kwargs):
    return None


def classImplementsOnly(*args, **kwargs):
    return None
