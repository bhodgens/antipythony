# Cites: S-02-15 S-01-27
# Pins: first true branch runs; later conditions not evaluated.
def classify(x):
    if x < 0:
        return 'neg'
    elif x == 0:
        return 'zero'
    elif x < 10:
        return 'small'
    else:
        return 'big'
print(classify(-5), classify(0), classify(3), classify(99))
