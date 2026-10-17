# Cites: S-05-4 (row itertools), anchor #7
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import itertools
import operator
names = ["accumulate", "batched", "chain", "combinations",
         "combinations_with_replacement", "compress", "count", "cycle",
         "dropwhile", "filterfalse", "groupby", "islice", "pairwise",
         "permutations", "product", "repeat", "starmap", "takewhile", "tee",
         "zip_longest"]
print(len(names), all(hasattr(itertools, n) for n in names))
print(list(itertools.islice(itertools.count(2), 3)),
      list(itertools.chain([1], [2])), list(itertools.batched("abc", 2)))
print(list(itertools.islice(itertools.count(1, 2), 4)))
print(list(itertools.islice(itertools.cycle("ab"), 5)))
print(list(itertools.repeat(7, 3)))
print(list(itertools.chain.from_iterable([[1], [], [2, 3]])))
print(list(itertools.accumulate([1, 2, 3, 4])))
print(list(itertools.accumulate([1, 2, 3, 4], operator.mul)))
print(list(itertools.compress("abcde", [1, 0, 1, 0, 1])))
print(list(itertools.dropwhile(lambda x: x < 3, [1, 2, 3, 1])))
print(list(itertools.takewhile(lambda x: x < 3, [1, 2, 3, 1])))
print(list(itertools.filterfalse(lambda x: x % 2, range(6))))
print(list(itertools.starmap(pow, [(2, 3), (3, 2)])))
print(list(itertools.pairwise("abcd")))
print(list(itertools.islice(itertools.tee(itertools.count(), 2)[1], 3)))
print(list(itertools.product("ab", [1, 2])))
print(list(itertools.product("ab", repeat=2)))
print(list(itertools.permutations([1, 2, 3], 2)))
print(list(itertools.combinations([1, 2, 3, 4], 2)))
print(list(itertools.combinations_with_replacement("ab", 2)))
print([f"{k}:{list(g)}" for k, g in itertools.groupby("aaabbc")])
print(list(itertools.zip_longest([1, 2], ["a"], fillvalue=0)))
print(list(itertools.islice(itertools.chain.from_iterable(
      itertools.repeat([1], 2)), 4)))
print(type(itertools.count(1)).__name__, len(list(itertools.permutations([]))),
      list(itertools.combinations([1], 5)))
