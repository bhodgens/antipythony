# T-01 valid fixture: walrus inside comprehensions (binding leaks to outer scope per 3.8 grammar).
# Cites: C-03 grammar; S-02-5 (comprehension implicit scope is where the := target binds outward from).
raw = "  spaced  "
stream = [cleaned := s.strip() for s in raw.split()]
limited = [y for x in range(10) if (y := x * x) < 30]
print(stream, cleaned, limited)
