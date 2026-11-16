# Cites: S-02-32
# try/except/else: else runs only when no exception fired.
try:
    x = 1
except ValueError:
    print("no")
else:
    print("else ran")
