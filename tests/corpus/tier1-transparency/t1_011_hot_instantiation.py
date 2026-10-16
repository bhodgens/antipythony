# Cites: S-03-05 (type.__call__ dispatch), S-03-06 (__new__/__init__ order), S-03-10 (setattr), C-31
# Stresses: 20k calls through one type-call site + instance __setattr__ specialization;
# heap-shape growth past the inline-cache threshold forces deopt mid-run.
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y

total = 0
for i in range(20000):
    p = Point(i, i * 2)
    p.x = p.x + 1
    total += p.x + p.y
print(total)
print(Point.__name__)
