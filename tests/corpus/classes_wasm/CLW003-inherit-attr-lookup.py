# Cites: S-03-9, S-03-5
# Attribute lookup walks the class mro; subclass inherits __init__.
class Base:
    def __init__(self, v):
        self.v = v

class Sub(Base):
    pass

s = Sub(9)
print(s.v)
