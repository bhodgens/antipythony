total = sum(x * x for x in range(10))
chunked = list((i, chr(i)) for i in range(5) if i % 2)
lazy = (n for n in range(100) if n % 7 == 0)
