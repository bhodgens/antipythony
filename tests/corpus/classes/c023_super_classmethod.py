# Cites: S-03-19
# super() in a classmethod body resolves with cls propagating (B->B).
class Ac:
    @classmethod
    def m(cls):
        return cls.__name__
class Bc(Ac):
    @classmethod
    def m(cls):
        return "B->" + super().m()
print(Bc.m())
class Cc(Bc): pass
print(Cc.m())
