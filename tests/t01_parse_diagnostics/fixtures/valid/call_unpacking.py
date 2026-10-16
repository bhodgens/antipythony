# T-01 valid fixture: iterable and mapping unpacking at the call site.
# Cites: S-02-9 (call-site positional/keyword unpacking, left-to-right evaluation).
def f(*args, **kwargs):
    return args, kwargs
pre = [1, 2]
post = (3,)
mid = {"x": 10}
end = {"y": 20, **mid}
r = f(*pre, *post, **mid, **end)
lst = [*pre, *post]
tup = (*pre, *post)
setx = {*pre, *post}
dct = {**mid, "z": 1}
print(r, lst, tup, setx, dct)
