# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-30
# Cites: S-02-32
# Cites: S-02-33
# Cites: S-02-28
# try/except/else/finally: handler var deleted at handler exit (S-02-30); else skipped when handled (S-02-32); finally on every path (S-02-33); bare raise keeps original traceback (S-02-28).
def h(flag):
    try:
        if flag:
            raise ValueError("boom")
        else:
            raise TypeError("other")
    except ValueError as e:
        first = "caught:" + str(e)
        raise
    finally:
        print("fin")
    return first
print(h(True))
