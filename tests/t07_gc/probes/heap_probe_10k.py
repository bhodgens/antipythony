# Impl-side heap-bounding probe (NOT a corpus case). Cites: C-20, C-21, D1.
# Leaf 02.04 gate: "no leak across 10k iterations (heap growth bounded)".
# Creates and drops 10,000 finalized instances interleaved with cyclic
# garbage, forces gc.collect(), and prints the exactly-once projection
# (same shape as corpus case gc014 at 2000). A correct tracing GC must
# keep this process's RSS bounded: the test harness measures the child
# max-RSS delta and the exit code. The projection itself is D1-safe: all
# sampling happens after explicit gc.collect().
import gc

count = 0
next_tok = 0
tokens = []

class S:
    def __del__(self):
        global count
        count += 1
        tokens.append(self.tok)

def spam(n):
    global next_tok
    for _ in range(n):
        s = S()
        s.tok = next_tok
        next_tok += 1
        s.link = [next_tok, next_tok + 1]
    for j in range(200):
        x = {"junk": [j]}
        x["me"] = x

for round_no in range(10):
    spam(1000)
    gc.collect()

print("fired:", count)
print("unique tokens:", len(set(tokens)))
print("complete:", sorted(tokens) == list(range(10000)))
