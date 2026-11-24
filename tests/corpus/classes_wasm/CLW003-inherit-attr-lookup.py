# Cites: S-03-09, S-03-05
# Attribute lookup walks the class mro; subclass inherits __init__.
class Base:
    def __init__(self, v):
        self.v = v

class Sub(Base):
    pass

s = Sub(9)
print(s.v)
