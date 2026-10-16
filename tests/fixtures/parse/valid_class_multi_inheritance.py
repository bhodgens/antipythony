class Base:
    def root(self):
        return "root"

class MixinA(Base):
    def name(self):
        return "a"

class MixinB:
    def tag(self):
        return "b"

class Combined(MixinA, MixinB, Base):
    def all(self):
        return (self.name(), self.tag(), self.root())
