# Cites: S-02-1 (LEGB), S-02-3 (global), S-02-30 (except dispatch), S-02-33 (finally), C-31
# Stresses: exception-driven deopt to tier-0 exact mid-state, 5 times across a 40k loop:
# loop counter, local accumulator, global, and finally-side-effects must all resume exactly.
G = 0

def churn():
    global G
    local_acc = 0
    thrown = 0
    i = 0
    while i < 40000:
        i += 1
        local_acc += i % 97
        if i % 8000 == 0:
            try:
                raise ValueError("mid-loop")
            except ValueError:
                thrown += 1
            finally:
                G += 1
    return local_acc, thrown

acc, thrown = churn()
print(acc, thrown, G)
