# Cites: S-01-14, S-06-3
# str + str concatenation through FN_BINOP (wasm-strplus defect 1); CPython
# TypeError on str + non-str stays a loud wasm trap on this slice (recorded
# divergence).
print('B' + 'A')
print('foo' + 'bar')
s = 'x'
print(s + 'y' + 'z')
print('' + '')
print('hello ' + 'world')
