# Leaf 1005a differential driver: the S1 container band.
# Runs byte-identical under antipythony (containers extension via build-ext)
# and CPython 3.14 (same source compiled against Python.h).
#
# Output contract:
# - Value paths print the Python-level repr of what the C surface returned
#   (byte-compares on both runtimes).
# - Error paths print TYPE-CLASS + MESSAGE where the message is runtime-
#   produced verbatim; KeyError prints CLASS ONLY (the ce S0/S1 error bridge
#   renders KeyError's message through the generic Str-arg path, which
#   double-quotes vs CPython's bare-key repr — a recorded ap-run/ce_bridge
#   finding; the CLASS identity and raise path are fully proven, the arg
#   rendering divergence is tracked separately, see NEXT-1005A.md).
import containers as c

# ---- dict: value paths ----
d = c.make_dict()
print("D1", d)
print("D2", c.dict_size(d))
print("D3", c.dict_keys(d))
print("D4", c.dict_values(d))
print("D5", c.dict_items(d))
print("D6", c.dict_getitem_ref(d, "a"))
print("D7", c.dict_getitem_ref(d, "zz"))
print("D8", c.dict_getitem_string_ref(d, "b"))
print("D9", c.dict_getitem_string_ref(d, "zz"))
print("D10", c.dict_getitem_masked(d, "a"))
print("D11", c.dict_getitem_masked(d, "zz"))
print("D12", c.dict_getitem_with_error(d, "b"))
print("D13", c.dict_getitem_with_error(d, "zz"))
print("D14", c.dict_contains(d, "a"))
print("D15", c.dict_contains(d, "zz"))
print("D16", c.dict_contains_string(d, "a"))
print("D17", c.dict_contains_string(d, "zz"))
print("D18", c.dict_pop(d, "a"))
print("D19", c.dict_pop(d, "zz"))
print("D20", c.dict_pop_string(d, "b"))
print("D21", c.dict_pop_string(d, "zz"))
print("D22", c.dict_size(d))
print("D23", c.dict_setdefault(d, "k1", 10))
print("D24", c.dict_setdefault(d, "k1", 99))
print("D25", c.dict_setdefault_ref(d, "k2", 20))
print("D26", c.dict_setdefault_ref(d, "k2", 88))
print("D27", c.dict_size(d))

d2 = c.make_dict()
print("D28", c.dict_delitem(d2, "a"))
print("D29", c.dict_delitem_string(d2, "b"))
print("D30", c.dict_size(d2))

d3 = c.make_dict()
cp = c.dict_copy(d3)
print("D31", cp, c.dict_size(d3))
c1, c2 = c.dict_copy_then_mut(d3)
print("D32", c1, c2)

d4 = c.make_dict()
print("D33", c.dict_merge(d4, c.make_dict(), 1))
d5 = c.make_dict()
print("D34", c.dict_merge(d5, c.make_dict(), 0), d5)
d6 = c.make_dict()
print("D35", c.dict_update(d6, c.make_dict()))

d7 = c.make_dict()
pairs = [("x", 1), ("y", 2)]
print("D36", c.dict_merge_seq2(d7, pairs, 1))
d8 = c.make_dict()
print("D37", c.dict_merge_seq2(d8, pairs, 0), d8)

d9 = c.make_dict()
print("D38", c.dict_next_pairs(d9))
d10 = c.make_dict()
print("D39", c.dict_clear(d10))

# ---- dict: error paths (class + verbatim runtime message) ----


def probe(fn, *args):
    try:
        r = fn(*args)
        print("OK", r)
    except Exception as e:
        if type(e).__name__ == "KeyError":
            print("ERR", type(e).__name__)
        else:
            print("ERR", type(e).__name__, str(e))


dE = c.make_dict()
probe(c.dict_setitem, dE, [1], 5)
probe(c.dict_delitem, dE, [1])
probe(c.dict_contains, dE, [1])
probe(c.dict_setdefault, dE, [1], 5)
probe(c.dict_getitem_with_error, dE, [1])
probe(c.dict_getitem_ref, dE, [1])
probe(c.dict_pop, dE, [1])
probe(c.dict_delitem, dE, "missing-key-zz")
probe(c.dict_delitem_string, dE, "missing-key-zz")

# wrong-typed receivers: the line-pinned SystemError dialect
# (PyList_Append + PyTuple_{Size,GetItem,SetItem} wrong-receiver dialects
# are S0-owned in exports.rs with a different text - recorded as findings,
# exercised on valid receivers only).
i5 = 5
probe(c.dict_size_bad, i5)
probe(c.dict_keys_bad, i5)
probe(c.dict_values_bad, i5)
probe(c.dict_items_bad, i5)
probe(c.dict_copy_bad, i5)
probe(c.dict_delitem_bad, i5, "k")
probe(c.dict_getitem_bad, i5, "k")
probe(c.dict_setitem_bad, i5, "k", 1)
probe(c.dict_setdefault_bad, i5, "k", 1)
probe(c.dict_pop_bad, i5, "k")
probe(c.dict_getitemref_bad, i5, "k")

# Merge/Update error shapes
probe(c.dict_merge, dE, [1, 2], 1)
probe(c.dict_update_pairs, c.make_dict(), i5)
probe(c.dict_update_pairs, c.make_dict(), "ab")
probe(c.dict_merge_seq2, c.make_dict(), 42, 1)
probe(c.dict_merge_seq2, c.make_dict(), [("a", 1, 2)], 1)
probe(c.dict_merge_seq2, c.make_dict(), [("a",)], 1)
probe(c.dict_merge_seq2, c.make_dict(), [42], 1)
probe(c.dict_merge_seq2, c.make_dict(), c.make_dict(), 1)

# ---- list: value paths ----
l0 = c.make_list()
print("L1", l0)
print("L2", c.list_size(l0))
print("L3", c.list_getitem(l0, 0))
print("L4", c.list_getitem_ref(l0, 2))
print("L5", c.list_getslice(l0, 1, 3))
print("L6", c.list_getslice(l0, -100, 100))
print("L7", c.list_getslice(l0, 2, 1))
print("L8", c.list_getslice(l0, 5, 100))

l1 = c.make_list()
c.list_setitem(l1, 1, 99)
print("L9", l1)

l2 = c.make_list()
print("L10", c.list_append(l2, 4))
print("L11", c.list_insert(l2, 0, 0))
print("L12", c.list_insert(l2, 100, 8))
print("L13", c.list_insert(l2, -100, -1))

l3 = c.make_list()
print("L14", c.list_extend(l3, (7, 8)))
print("L15", c.list_extend(l3, "ab"))
print("L16", c.list_size(l3))

l4 = c.make_list()
print("L17", c.list_setslice(l4, 1, 2, (9, 9)))
l5 = c.make_list()
print("L18", c.list_setslice(l5, 0, 1, ()))
l6 = c.make_list()
print("L19", c.list_setslice(l6, 1, 0, (5,)))

l7 = c.make_list()
print("L20", c.list_reverse(l7))
print("L21", c.list_sort(l7))
l8 = [3, 1, 2]
print("L22", c.list_sort(l8))
ls = ["b", "a", "c"]
print("L23", c.list_sort(ls))

l9 = c.make_list()
print("L24", c.list_as_tuple(l9))
l10 = c.make_list()
print("L25", c.list_clear(l10))

# ---- list: error paths ----
probe(c.list_getitem, c.make_list(), 5)
probe(c.list_getitem, c.make_list(), -1)
probe(c.list_getitem_ref, c.make_list(), 5)
probe(c.list_setitem, c.make_list(), 5, 1)
probe(c.list_setitem, c.make_list(), -1, 1)
probe(c.list_setslice, c.make_list(), 0, 1, 42)
probe(c.list_extend, c.make_list(), 42)
probe(c.list_size_bad, i5)
probe(c.list_sort_bad, i5)
probe(c.list_reverse_bad, i5)
probe(c.list_astuple_bad, i5)
probe(c.list_getitem_bad, i5, 0)
probe(c.list_setitem_bad, i5, 0, 1)
probe(c.list_insert_bad, i5, 0, 1)
probe(c.list_getslice_bad, i5)
probe(c.list_clear_bad, i5)
ml = c.make_list()
ml.append(1)
ml.append("a")
probe(c.list_sort, ml)

# ---- tuple: value paths ----
t0 = c.make_tuple()
print("T1", t0)
print("T2", c.tuple_size(t0))
print("T3", c.tuple_getitem(t0, 0))
print("T4", c.tuple_getitem(t0, 2))
print("T5", c.tuple_getslice(t0, 1, 3))
print("T6", c.tuple_getslice(t0, -100, 100))
print("T7", c.tuple_getslice(t0, 2, 1))
print("T8", c.tuple_getslice(t0, -2, 2))
print("T9", c.tuple_getslice(t0, 1, -100))
print("T10", c.tuple_pack(t0, 42))

tt = ["", "", ""]
probe(c.tuple_setitem_dense, tt, 0, "dense")
print("T11", tt)

# ---- tuple: error paths ----
probe(c.tuple_getitem, t0, 5)
probe(c.tuple_getslice_bad, i5)

# ---- refcount contract (steal/borrow) ----
rc = c.rc_probe()
print("R1 [%d, %d, %d, %d]" % (rc[0], rc[1], rc[2], rc[3]))

print("DONE")
