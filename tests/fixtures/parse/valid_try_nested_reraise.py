def layered():
    try:
        try:
            raise KeyError("inner")
        finally:
            log = True
    except KeyError:
        raise
