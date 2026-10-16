def full_shape():
    try:
        value = int("10")
    except ValueError:
        value = 0
    else:
        value += 1
    finally:
        cleanup = True
    return value
