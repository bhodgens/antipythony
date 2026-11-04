"""antipythony deviation shim — zope.interface.adapter (AdapterRegistry).
v0 inert: registration is a no-op, lookup always misses (the file:// gate
never routes real adapters)."""


class AdapterRegistry:
    def __init__(self, *a, **k):
        self._map = {}

    @staticmethod
    def _key(required, provided, name):
        def nm(x):
            return getattr(x, "__name__", None) or repr(x)

        if isinstance(required, (list, tuple)):
            rk = tuple(nm(x) for x in required)
        else:
            rk = nm(required)
        return (rk, nm(provided), name)

    def register(self, required, provided, name, factory):
        self._map[self._key(required, provided, name)] = factory

    def registerAdapter(self, factory, *ifaces):
        pass

    def lookup(self, required, provided, name="", default=None):
        return default

    def registered(self, required, provided, name=""):
        return None

    def lookup1(self, required, provided, default=None):
        return default

    def queryAdapter(self, obj, provided, default=None):
        return default

    def adapter_hook(self, provided, obj, name="", default=None):
        return default

    def lookupAll(self, required, provided):
        return ()

    def subscriptions(self, required, provided):
        return ()

    def subscribe(self, required, provided, factory):
        pass
