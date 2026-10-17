cname = "myvar"
s = '"#define X {" f"{cname};" "%s"'
print(s)
t = "a" f"b{x}" "c" f"{cname}" if (x := 9) else ""
print(t)
