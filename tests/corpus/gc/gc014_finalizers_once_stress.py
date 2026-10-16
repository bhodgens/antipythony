# Cites: C-21, D1
# C-21 stress shape (scaled down for the corpus; the 10k bounded-heap
# probe lives as an impl-side gate test): 2000 instances, each with a
# monotonic token, dropped into garbage interleaved with other garbage,
# one forced gc.collect(). Assert final count == dropped instances AND no
# object's __del__ fired twice, keyed by token (NOT id(), per C-21:
# moving heaps must not re-fire on slot reuse — tokens detect it).
import gc

count = 0
tokens = []

class S:
    def __del__(self):
        global count
        count += 1
        tokens.append(self.tok)

def spam(n):
    for i in range(n):
        s = S()
        s.tok = i
        s.link = [i, i + 1]
        cycle_head = s
    # interleaved real garbage
    for j in range(50):
        x = {"junk": [j]}
        x["me"] = x

spam(2000)
gc.collect()
print("fired:", count)
print("unique tokens:", len(set(tokens)))
print("tokens complete:", sorted(tokens) == list(range(2000)))
gc.collect()
print("no second round:", count)
