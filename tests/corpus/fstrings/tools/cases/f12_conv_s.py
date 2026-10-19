class C:
    def __str__(self): return "S"
    def __repr__(self): return "R"
print(f"[{C()!s}]|[{C()!r}]")
