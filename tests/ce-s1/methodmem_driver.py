# Leaf 1006 differential driver part 2: method/class ctors, PyMutex,
# Py_Version, PyUnstable markers, PyMem front, PyGILState bookkeeping.
# Byte-identical under antipythony (methodmem via build-ext) and CPython
# 3.14.7 (same source vs Python.h).
import methodmem as m

# M: method/class ctors
print("M1", m.m_method_new(7, None))
print("M2", m.m_cm_sm(7))
print("M3", m.m_type_tokens())

# K: PyMutex
print("K1", m.k_mutex())

# V: version (prefix + hex composite; the build-note suffix is
# compiler-dependent on both runtimes and not pinned)
print("V1", m.v_version())

# N: unstable markers
print("N1", m.n_unstable(int))

# P: PyMem front
print("P1", m.p_pymem())

# G: GILState bookkeeping
print("G1", m.g_gilstate())

print("DONE")
