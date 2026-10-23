# Vendored collections extensions (antipythany fix pass).
#
# Augments the native `collections` module (deque) with the dict subclasses
# the stdlib import surface needs (typing.py: `from collections import
# defaultdict`). Copied/adapted from CPython 3.14 collections (PSF-2.0; see
# NOTICE.md). v0: get/setdefault mirror dict semantics; the mutation-tracking
# extras are unexercised.
#
# s015 note: Counter/OrderedDict pure-python implementations live here too,
# but the native module registers its own interned types for those names and
# the augmentation rule only copies names the native namespace lacks — so the
# native fallbacks win. The interp's dict-subclass ctor/super() gaps (outside
# this file's scope) block the pure-python versions for now.

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


class Counter(dict):
    """Dict subclass for counting hashable items (CPython 3.14 subset)."""

    def __init__(self, iterable=None, **kwargs):
        dict.__init__(self)
        self.update(iterable, **kwargs)

    def __missing__(self, key):
        return 0

    def update(self, iterable=None, **kwargs):
        if iterable is not None:
            if hasattr(iterable, 'items'):
                for k, v in iterable.items():
                    self[k] = v
            else:
                for k in iterable:
                    self[k] = self.get(k, 0) + 1
        for k, v in kwargs.items():
            self[k] = v

    def most_common(self, n=None):
        items = sorted(self.items(), key=lambda kv: kv[1], reverse=True)
        if n is not None:
            return items[:n]
        return items

    def total(self):
        return sum(self.values())

    def __repr__(self):
        if not self:
            return '%s()' % (self.__class__.__name__,)
        return '%s(%r)' % (self.__class__.__name__, dict(self))

    def __sub__(self, other):
        out = self.__class__()
        for k in self:
            nv = self[k] - other.get(k, 0)
            if nv > 0:
                out[k] = nv
        return out


class OrderedDict(dict):
    """Dict that remembers insertion order (v0 dict is insertion-ordered)."""

    def __init__(self, iterable=None, **kwargs):
        dict.__init__(self)
        if iterable is not None:
            if hasattr(iterable, 'items'):
                for k, v in iterable.items():
                    self[k] = v
            else:
                for k, v in iterable:
                    self[k] = v
        for k, v in kwargs.items():
            self[k] = v

    def __repr__(self):
        if not self:
            return '%s()' % (self.__class__.__name__,)
        return '%s(%r)' % (self.__class__.__name__, dict(self))

    def move_to_end(self, key, last=True):
        if key not in self:
            raise KeyError(key)
        val = self[key]
        del self[key]
        if last:
            self[key] = val
        else:
            items = [(key, val)] + list(self.items())
            self.clear()
            for k, v in items:
                self[k] = v

    def popitem(self, last=True):
        if not self:
            raise KeyError('popitem(): dictionary is empty')
        items = list(self.items())
        k, v = items[-1] if last else items[0]
        del self[k]
        return (k, v)
