matrix = [[r * c for c in range(3)] for r in range(3)]
flat = [v for row in matrix for v in row]
conditional = [x if x > 0 else -x for x in (-2, 3, -4)]
