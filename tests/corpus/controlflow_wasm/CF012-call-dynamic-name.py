# Cites: S-07-23
# Pins: functions held in variables / nested defs ride the generic call
# path; callable-valued names dispatch to the same behavior.
def base(x):
    return x * 2
alias = base
print(alias(21))
def outer():
    def inner(y):
        return y + 1
    return inner(41)
print(outer())
