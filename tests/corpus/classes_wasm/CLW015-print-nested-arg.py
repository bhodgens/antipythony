# Cites: S-06-3
# print argument evaluation that itself prints (wasm-strplus defect 2): the
# inner line must appear first, then the OUTER line complete and unclobbered.
def inner():
    print('inner')
    return 'B'
print('A', inner())
print('X', inner(), 'Y')
