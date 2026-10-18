# Vendored collections extensions (antipythany fix pass).
#
# Augments the native `collections` module (deque) with the dict subclasses
# the stdlib import surface needs (typing.py: `from collections import
# defaultdict`). Copied/adapted from CPython 3.14 collections (PSF-2.0; see
# NOTICE.md). v0: get/setdefault mirror dict semantics; the mutation-tracking
# extras are unexercised.

__all__ = ['defaultdict']


class defaultdict(dict):
    """dict subclass that calls a factory function to supply missing values."""

    def __init__(self, default_factory=None, **kwargs):
        super().__init__(**kwargs)
        self.default_factory = default_factory

    def __missing__(self, key):
        if self.default_factory is None:
            raise KeyError(key)
        self[key] = value = self.default_factory()
        return value

    def get(self, key, default=None):
        try:
            return self[key]
        except KeyError:
            return default

    def setdefault(self, key, default=None):
        try:
            return self[key]
        except KeyError:
            self[key] = default
        return default

    def __repr__(self):
        if not self:
            return '%s()' % (self.__class__.__name__,)
        return '%s(%r, %r)' % (self.__class__.__name__, self.default_factory, dict(self))

    def copy(self):
        return self.__class__(self.default_factory, **self)

    def __copy__(self):
        return self.copy()
