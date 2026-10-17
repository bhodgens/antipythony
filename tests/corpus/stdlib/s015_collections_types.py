# Cites: S-05-4 (row collections), anchor #8, C-26
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import collections
names = ["namedtuple", "deque", "defaultdict", "OrderedDict", "Counter",
         "ChainMap", "UserDict", "UserList", "UserString", "abc"]
print(len(names), all(hasattr(collections, n) for n in names))
print(collections.Counter("aab")["a"])
c = collections.Counter("banana")
print(c)
print(c.most_common())
print(c.most_common(2))
print(collections.Counter("aab") - collections.Counter("ab"))
print(sorted(c.items()))
print(collections.Counter(a=2, b=1))
print(collections.Counter("abab").total())
d = collections.deque([1, 2, 3], maxlen=4)
print(d)
d.append(4)
d.append(5)
print(d, len(d))
d.appendleft(0)
print(d)
d.rotate(1)
print(d, d.popleft(), d.pop(), list(d))
print(collections.deque("abcd").__class__.__name__,
      collections.deque([1, 2]).index(2))
dd = collections.defaultdict(list)
for ch in "hello":
    dd[ch].append(ord(ch) % 3)
print({k: dd[k] for k in sorted(dd)})
ddint = collections.defaultdict(int)
ddint["x"] += 1
print(ddint, ddint.default_factory is int)
od = collections.OrderedDict([("b", 1), ("a", 2)])
print(od)
od.move_to_end("b")
print(list(od.keys()), od.popitem(last=False))
print(od == {"a": 2, "b": 1}, list(od) == ["a", "b"])
Point = collections.namedtuple("Point", "x y")
pt = Point(1, 2)
print(pt, pt.x, pt[1], pt._replace(y=5), pt._asdict(), Point._fields)
print(tuple(pt), len(pt), Point(1, 2) == (1, 2), list(Point._make([3, 4])))
cm = collections.ChainMap({"a": 1}, {"a": 2, "b": 3})
print(cm["a"], cm["b"], len(cm.maps))
class Counted(collections.UserDict):
    def __setitem__(self, k, v):
        self.sets = getattr(self, "sets", 0) + 1
        super().__setitem__(k, v)
cd = Counted()
cd["z"] = 1
print(dict(cd), cd.sets)
print(collections.UserList([1]) + [2], collections.UserString("ab").upper())
print(issubclass(collections.Counter, dict),
      isinstance(collections.OrderedDict(), dict))
