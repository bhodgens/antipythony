# Cites: C-19, C-20, C-23, D1
# del-of-one-alias MUST NOT finalize the object (another name still holds
# it) under BOTH models: refcount-prompt destruction keeps it alive too
# (refcount is not zero) and GC-delayed destruction keeps it alive (it is
# reachable). Therefore the mid-run observation is timing-invariant. The
# total count is asserted only after an explicit gc.collect() (assert on
# total, not order — C-23). Under CPython refcounting the final __del__
# lands at `del keeper` (prompt, before the marker print); under
# antipythony it MAY land only at the following gc.collect() — both
# produce identical bytes because the marker print sits BEFORE the collect
# and the count is sampled AFTER it.
import gc

fired = []

class A:
    def __del__(self):
        fired.append("gone")

a = A()
b = a
del a
# One alias removed, one still refers: nothing may be finalized here in
# either model.
print("alias still holds:", len(fired), b is not None)

gc.collect()
print("no fire while aliased:", len(fired))

keeper = b
del b
del keeper  # refcount model: __del__ may fire here; GC model: not yet
gc.collect()  # both models: finalized by now (C-20 forced-finalization)
print("total fired by collect:", len(fired))
