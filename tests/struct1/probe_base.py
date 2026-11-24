import pydantic
print("VERSION", pydantic.VERSION)
from pydantic import BaseModel
print("IMPORTED", BaseModel.__name__)
