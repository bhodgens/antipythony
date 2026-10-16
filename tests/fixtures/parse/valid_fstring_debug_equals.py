value = 7
name = "x"
a = f"{value=}"
b = f"{value = }"
c = f"{name!r}"
d = f"{name!s:>{len(name)}}"
e = f"mixed {value=} and {name!r}"
