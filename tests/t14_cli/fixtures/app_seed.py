# T-14 scaffold seed — `antipythany new myapp` must produce a runnable
# myapp.py (Plan 04 §Design: "scaffold: myapp.py + antipythany.toml").
# This file is the SOURCE TEMPLATE the test copies into the temp project as
# the app program; once deps are added it imports the fixture dep, which
# proves the lock closure actually reaches the compile (leaf 04.01 Gate:
# "new/add/run flow with one pure-Python dep works end to end").
import tiny_pure

print(tiny_pure.greet("antipythany"))
