def http_status(code):
    match code:
        case 200:
            return "ok"
        case 404:
            return "missing"
        case _:
            return "other"
