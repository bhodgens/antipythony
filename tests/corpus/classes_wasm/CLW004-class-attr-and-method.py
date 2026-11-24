# Cites: S-03-03, S-03-09
# Class-level attribute via mro dict; method with no args beyond self.
class C:
    k = 42

    def get(self):
        return self.k

c = C()
print(c.k)
print(c.get())
