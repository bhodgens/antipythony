rows = [["a", 1], ["b", 2]]
a = f"{rows!r}"
b = f"{[str(cell) for cell in row]!s}"
c = f"{format(deep, '>4')}"
