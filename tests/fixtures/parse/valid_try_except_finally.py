def risky():
    try:
        return 1 / 0
    except ZeroDivisionError:
        return None
    finally:
        pass
