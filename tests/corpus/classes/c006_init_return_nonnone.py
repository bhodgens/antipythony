# Cites: S-03-08
# __init__ must return None.
class R8:
    def __init__(self):
        return 5
try:
    R8()
except TypeError as e:
    print("TypeError:", e)
class R8b:
    def __init__(self):
        return self
try:
    R8b()
except TypeError as e:
    print("TypeError:", e)
