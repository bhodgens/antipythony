# Cites: S-05-4 (row copy); C-19 (no finalize-order asserts)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import collections
import copy
names = ["copy", "deepcopy", "Error", "error", "dispatch_table", "replace"]
print(len(names), all(hasattr(copy, n) for n in names), copy.error is copy.Error)
xs = [[1, 2], {"a": [3]}]
sh = copy.copy(xs)
dp = copy.deepcopy(xs)
sh[0].append(99)
print(xs[0], dp[0], dp[1] is not xs[1], dp[1]["a"] == [3])
class Node:
    def __init__(self):
        self.kids = []
n = Node()
n.kids.append(n)
c2 = copy.deepcopy(n)
print(c2.kids[0] is c2)
NT = collections.namedtuple("NT", "a b")
print(copy.replace(NT(1, 2), a=9))
try:
    copy.deepcopy(len)
except TypeError as e:
    print("deepcopy-builtin:", str(e)[:40])
print(type(copy.copy([1, 2])) is list,
      copy.deepcopy(frozenset([1])) == frozenset([1]))
d = {1: [2]}
print(copy.deepcopy(d) == d, copy.deepcopy.__name__)
print(copy.copy(int) is int)
print(hasattr(copy, "dispatch_table"), isinstance(copy.Error, type))
