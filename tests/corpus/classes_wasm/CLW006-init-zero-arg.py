# Cites: S-03-4
# Zero-arg __init__ dispatched through cls_call.
class Z:
    def __init__(self):
        self.tag = "z"

z = Z()
print(z.tag)
