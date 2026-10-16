# Cites: C-20, C-21, C-23, S-06-17#1, D1
# __del__ runs at gc.collect(): property form (count-by-exit), never an
# interleaving assertion. Finalizer lines print via a counter list observed
# only AFTER an explicit gc.collect() (C-23 allowed form).
import gc

fired = []

class Noisy:
    def __del__(self):
        fired.append("del")

for _ in range(3):
    Noisy()

gc.collect()
# After an explicit collect all unreachable objects are finalized (C-20:
# "After an explicit gc.collect() ... the del lines MUST all have appeared").
print("fired_after_collect", len(fired))
print("loop done")
gc.collect()
print("fired_at_exit", len(fired))
