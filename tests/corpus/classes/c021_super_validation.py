# Cites: S-03-17
# Two-arg super() validation; bare super() outside a method body.
class SA: pass
class SB(SA): pass
try:
    super(SA, 5)
except TypeError as e:
    print("TypeError:", e)
try:
    super(SB, SA())
except TypeError as e:
    print("TypeError:", e)
print(isinstance(super(SA, SB()), SA), type(super(SA, SB())).__name__)
class Holder(SA):
    def probe(self):
        def inner():
            return super()
        return inner()
try:
    Holder().probe()
except RuntimeError as e:
    print("RuntimeError:", e)
