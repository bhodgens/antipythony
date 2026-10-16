# T-05 values corpus V029
# Cites: S-01-29
# Pins: `not` ALWAYS returns a real bool (never the operand), including for
# objects with a custom truthiness protocol.
print(not 0, not not 0, not "x", not [], not None, not 1)
print(type(not 0).__name__, type(not "x").__name__, not not None)
class L:
    def __len__(self):
        return 0
print(not L())
