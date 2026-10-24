"""antipythany deviation shim — zope.interface.interface module surface.
v0 inert (see _shim.py, docs/lxml-shim.md)."""


class Attribute:
    def __init__(self, name, type_name=None):
        self.__name__ = name
        self.type_name = type_name


class SpecificationBase:
    def __iter__(self):
        return iter(self.names(all=True))


class InterfaceClass(SpecificationBase):
    _attrs = None

    def __init__(self, name=None, bases=(), attrs=None, __doc__="",
                 __module__=None):
        self.__name__ = name
        self.__doc__ = __doc__ or ""
        self.__bases__ = tuple(bases)
        self.__attrs = {}
        if attrs:
            for k, v in attrs.items():
                if not k.startswith("_"):
                    self.__attrs[k] = Attribute(k)

    def __call__(self, obj, alternate=None):
        return False

    def providedBy(self, obj):
        return False

    def implementedBy(self, cls):
        return False

    def isOrExtends(self, other):
        return False

    def names(self, all=False):
        if not all:
            return self.__attrs.keys()
        r = set(self.__attrs.keys())
        for base in self.__bases__:
            r.update(base.names(all=True))
        return iter(r)

    def namesAndDescriptions(self, all=False):
        if not all:
            return self.__attrs.items()
        r = {}
        for base in self.__bases__[::-1]:
            r.update(dict(base.namesAndDescriptions(all)))
        r.update(self.__attrs)
        return r.items()

    def get(self, name):
        return self.__attrs.get(name)

    def __contains__(self, name):
        return name in self.names(all=True)

    def __getitem__(self, name):
        return self.__attrs[name]


class Interface:
    def __init__(self, name=None, bases=()):
        self.__name__ = name

    def __call__(self, obj, alternate=None):
        return False

    def providedBy(self, obj):
        return False

    def implementedBy(self, cls):
        return False


adapter_hooks = []


def _obsolete(callable_):
    return callable_
