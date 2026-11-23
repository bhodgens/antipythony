# Leaf 1005c differential driver part 1: the PyErr band.
# (errtb_driver2.py holds the frame/code/gen projections; the two-part
# split pins each band deterministically — the combined run has a recorded
# intermittent abort, see NEXT-1005C.md findings.)
import errtb

print("custom bases:", errtb.custom.__bases__)
print("custom_sub bases:", errtb.custom_sub.__bases__)
print("custom_sub doc:", errtb.custom_sub.__dict__.get("__doc__"))

# E1: SetString -> Fetch -> Restore -> NULL -> Python catches same class.
try:
    errtb.raise_catch("round trip", ValueError)
except ValueError as e:
    print("E1", type(e).__name__, e)

# E1b: the exception CLASS rides the Fetch triple too.
try:
    errtb.raise_catch("lookup trip", KeyError)
except LookupError as e:
    print("E1b", type(e).__name__, e)

# E2: PyErr_Format %S %d (str of a str obj + int).
try:
    errtb.fmt_S_d("thing", 42)
except ValueError as e:
    print("E2", type(e).__name__, e)

# E3: PyErr_Format %S (str form; CPython's bare %s reads encoded-object
# internals whose rendering is not reproducible — %S is the byte-exact
# PyObject_Str conversion on both runtimes).
try:
    errtb.fmt_s(7)
except TypeError as e:
    print("E3", type(e).__name__, e)

# E4: PyErr_Format %zd (ssize_t).
try:
    errtb.fmt_zd(1234567890)
except ValueError as e:
    print("E4", type(e).__name__, e)

# E5: ExceptionMatches on the current error (subclass hierarchy).
print("E5 IndexError-is-LookupError:", errtb.matches(LookupError))
print("E5b IndexError-is-TypeError:", errtb.matches(TypeError))
print("E5c IndexError-is-IndexError:", errtb.matches(IndexError))

# E6: GivenExceptionMatches on a real instance.
inst = ValueError("probe")
print("E6 val-is-ValueError:", errtb.matches_given(inst, ValueError))
print("E6b val-is-Exception:", errtb.matches_given(inst, Exception))
print("E6c val-is-TypeError:", errtb.matches_given(inst, TypeError))

# E7: GetRaisedException/SetRaisedException round-trip.
try:
    errtb.get_set_raised("moved out and back")
except RuntimeError as e:
    print("E7", type(e).__name__, e)

# E8: SetObject with the message riding the value slot.
try:
    errtb.setobject("explicit instance")
except ValueError as e:
    print("E8", type(e).__name__, e)

# E9: NormalizeException + the normalized val's given-match probe.
try:
    errtb.normalize("normalized")
except ValueError as e:
    print("E9", type(e).__name__, e)

# E10: NewException class raised INTO Python; MRO + catchability.
try:
    raise errtb.custom("custom raised")
except errtb.custom as e:
    print("E10", type(e).__name__, e, issubclass(errtb.custom, Exception))

# E10b: the WITH-DOC subclass catches via the TRANSITIVE base.
try:
    raise errtb.custom_sub("sub raised")
except errtb.custom as e:
    print("E10b", type(e).__name__, e)

print("DONE")
