# Same-quote nesting inside the replacement field (PEP 701, 3.12+)
x = "inner"
print(f"outer {x} {"lit"} end")
