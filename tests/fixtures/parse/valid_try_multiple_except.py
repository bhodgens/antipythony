def parse_pair(s):
    try:
        a, b = s.split(",")
        return int(a), int(b)
    except (ValueError, TypeError):
        return (0, 0)
    except AttributeError as exc:
        raise RuntimeError("bad input") from exc
