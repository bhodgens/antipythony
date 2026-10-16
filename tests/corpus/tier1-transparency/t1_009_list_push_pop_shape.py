# Cites: S-01-19 (list mutation), S-02-19 (starred unpack), C-31
# Stresses: append/pop/clear/extend on one list drive repeated shape changes under a hot
# method-IC site; starred unpack in the loop pins per-iteration element counts.
st = []
events = 0
for i in range(20000):
    st.append(i)
    if i % 3 == 0:
        head, *rest = [st.pop(), len(st)]
        st.extend(rest)
        events += head
    if i % 1000 == 999 and st:
        st.clear()
        st.append(-1)
print(len(st))
print(events)
print(st[-5:])
