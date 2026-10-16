lengths = {w: len(w) for w in ["a", "bb", "ccc"]}
filtered = {k: v for k, v in lengths.items() if v > 1}
swapped = {v: k for k, v in lengths.items()}
