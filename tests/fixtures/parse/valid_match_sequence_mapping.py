def shape(v):
    match v:
        case {"kind": "point", "x": x, "y": y}:
            return (x, y)
        case {"kind": "circle", **rest}:
            return rest
        case [x, y] | (x, y):
            return (x, y)
