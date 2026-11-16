# Cites: S-02-27, S-02-30
# raise builtin exc with message; except binds; args tuple; execution continues.
try:
    raise ValueError("boom")
except ValueError as e:
    print("caught", e.args)
print("after")
