# T-01 valid fixture: try/except multiple clauses/else/finally, exception groups of names, `from` re-raise.
# Cites: S-02-30/32/33 (try/except/else/finally structure); plan-01 Stmt list names `try`.
def run(flag):
    try:
        if flag:
            raise ValueError("bad")
    except (TypeError, ValueError) as e:
        out = f"caught {type(e).__name__}"
    except Exception:
        out = "other"
        raise RuntimeError("wrapped") from None
    else:
        out = "clean"
    finally:
        out += "|fin"
    return out
print(run(True))
