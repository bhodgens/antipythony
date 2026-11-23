# NEXT-PYDANTIC-PROBE driver: the kill-criterion #3 gate (survey §3).
# import pydantic_core; the FromSpec-built Model; instantiate; validate one
# field — plus the measured-169-symbol demand self-report. The antipythony
# arm prints tagged lines; the CPython arm (APX_CPYTHON_BUILD) must produce
# byte-identical stdout for the GATE/RAISE/EXC/probe-surface lines (the
# <__main__.Model object> repr line is arm-formatted and excluded).
import pydantic_core as pc

# Gate step 1: the module IMPORTED (reaching this line proves dlopen +
# PyInit + the full init path: PyModule_Create, PyErr_NewException,
# PyModule_AddObject, PyType_FromSpec).

# Gate step 2: the FromSpec-built type is exposed and callable.
print("MODEL", type(pc.Model).__name__)

# Gate step 3: instantiate + validate one field (the survey §3 gate).
inst = pc.Model(42)
print("GATE", inst.x)

# Validation error paths (the ValueError/TypeError dialects).
try:
    pc.Model(-1)
    print("NEG_MISSED")
except ValueError as e:
    print("NEG_RAISED", str(e)[:45])
try:
    pc.Model("not an int")
    print("TYPE_MISSED")
except TypeError as e:
    print("TYPE_RAISED", str(e)[:50])

# ValidationError class exposed (PyErr_NewException member).
print("EXC", pc.ValidationError.__name__)

# The measured-demand self-report (each printf line exercises members
# from docs/ce-scan-symbols-full.json, pydantic_core-2.50.5 key).
pc.probe_surface()

print("PROBE_DONE")
