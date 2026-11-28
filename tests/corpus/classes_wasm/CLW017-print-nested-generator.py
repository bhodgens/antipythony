# Cites: S-06-3
# same shape through a generator driven inside the argument list (defect 2).
def gen():
    print('gen-inner')
    yield 'G'
def take(it):
    return next(it)
print('A', take(gen()))
print('done')
