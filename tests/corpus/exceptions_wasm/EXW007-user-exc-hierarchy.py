# Cites: S-03-28, S-02-30
# User exception hierarchy: except Parent catches Child instance.
class Parent(Exception):
    pass

class Child(Parent):
    pass

try:
    raise Child("c")
except Parent:
    print("parent caught")
