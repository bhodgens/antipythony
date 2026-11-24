import importlib
print("import_module is:", type(importlib.import_module), getattr(importlib.import_module, "__module__", None))
m = importlib.import_module('.sub', package='P')
print("GOT:", m.OUTPUT)
