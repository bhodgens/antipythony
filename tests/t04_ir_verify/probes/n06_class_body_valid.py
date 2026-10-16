class C:
    x = 10
    y = x + 1
    def m(self):
        return self.x

c = C()
print(c.m(), C.y)
