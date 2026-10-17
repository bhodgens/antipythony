# Cites: S-05-4 (row collections.abc)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import collections.abc as ca
names = ["Awaitable", "Callable", "Collection", "Container", "Coroutine",
         "Generator", "Hashable", "Iterable", "Iterator", "Reversible",
         "AsyncGenerator", "AsyncIterable", "AsyncIterator", "ByteString",
         "KeysView", "ItemsView", "ValuesView", "Mapping", "MappingView",
         "MutableMapping", "MutableSequence", "MutableSet", "Sequence",
         "Set", "Sized", "ABCMeta", "abstractmethod"]
print(len(names), all(hasattr(ca, n) for n in names))
print(ca.Hashable.__name__, isinstance(ca.Hashable, ca.ABCMeta))
checks = [
    isinstance([], ca.Sequence), isinstance((), ca.Sequence),
    isinstance("s", ca.Sequence), isinstance({}, ca.Mapping),
    isinstance({}, ca.MutableMapping), isinstance(set(), ca.MutableSet),
    isinstance(frozenset(), ca.Set), isinstance({}, ca.Collection),
    isinstance(iter([]), ca.Iterator), isinstance([1], ca.Iterable),
    isinstance([1], ca.Reversible), isinstance(len, ca.Callable),
    isinstance("s", ca.Sized), isinstance(b"s", ca.ByteString),
    isinstance({}.keys(), ca.KeysView), isinstance({}.items(), ca.ItemsView),
    isinstance({}.values(), ca.ValuesView),
    not isinstance(int(), ca.Iterable),
]
print(all(checks), len(checks))
class MySeq(ca.Sequence):
    def __init__(self, xs):
        self.xs = xs
    def __getitem__(self, i):
        return self.xs[i]
    def __len__(self):
        return len(self.xs)
ms = MySeq([5, 6, 7])
print(list(ms), ms[1:], ms.index(6), 6 in ms, len(ms))
class MyMap(ca.Mapping):
    def __init__(self, d):
        self.d = d
    def __getitem__(self, k):
        return self.d[k]
    def __iter__(self):
        return iter(self.d)
    def __len__(self):
        return len(self.d)
print(sorted(MyMap({"a": 1, "b": 2})), sorted(MyMap({"a": 1}).items()))
print(isinstance(dict, ca.ABCMeta))
