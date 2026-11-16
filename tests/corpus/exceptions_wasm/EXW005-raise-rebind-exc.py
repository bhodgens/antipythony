# Cites: S-02-28
# Re-raise a bound exception from inside the handler; outer catches it.
try:
    try:
        raise ValueError("v")
    except ValueError as e:
        raise e
except ValueError as e2:
    print("re", e2.args)
