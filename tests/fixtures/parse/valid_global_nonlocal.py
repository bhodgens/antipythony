counter = 0

def bump():
    global counter
    counter += 1

def make_accumulator():
    total = 0
    def add(n):
        nonlocal total
        total += n
        return total
    return add
