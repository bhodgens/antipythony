# Cites: S-03-04, S-03-28
# User exception class derived from Exception: instantiate + raise + catch + args.
class MyErr(Exception):
    pass

try:
    raise MyErr("custom")
except MyErr as e:
    print("caught", e.args)
