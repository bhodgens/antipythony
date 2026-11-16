# Cites: S-02-30
# Nested try: inner handler catches; outer body continues.
try:
    try:
        raise ValueError("inner")
    except ValueError:
        print("inner caught")
    print("between")
except ValueError:
    print("outer caught")
