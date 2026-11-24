# F-STRUCT-1 gate driver — the exact NEXT-T3 S2 repro bytes, trimmed to the
# pydantic gate (S3/S4/S5 are other leaves' walls: vendored gaps + fileno).
# Run: APX_EXT_DIR=<ce-deps> antipythony run gate_pydantic.py
import core_schema_shim  # noqa: F401  (upgrades pydantic_core.core_schema)
import pydantic_core  # noqa: F401  (S0: must be importable first)

print("STAGE S2 from pydantic import BaseModel")
from pydantic import BaseModel


class M(BaseModel):
    x: int


print("STAGE S2-instantiate")
m = M(x=1)
print("S2 OK", m.x)
print("DONE")
