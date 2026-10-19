# str f-string adjacent to bytes literal: SyntaxError in CPython
x = "s"
print(f"{x}" b"tail")
