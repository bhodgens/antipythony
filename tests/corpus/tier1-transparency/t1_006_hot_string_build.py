# Cites: S-01-14 (str immutability), S-01-16 (f-strings), S-02-17, C-31
# Stresses: string-IC specialization in a hot loop that both concatenates (+=) and appends
# formatted pieces to a list, then joins; lengths and sampled prefix pin every iteration ran once.
chunks = []
acc = ""
for i in range(20000):
    piece = f"{i % 97:02d}x"
    chunks.append(piece)
    if i % 8 == 0:
        acc += piece
s = "".join(chunks)
print(len(s))
print(len(acc))
print(s[:24])
print(acc[:24])
