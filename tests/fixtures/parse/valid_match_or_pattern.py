def route(cmd):
    match cmd.split():
        case ["quit" | "exit"]:
            return "bye"
        case ["load", path] | ["open", path]:
            return path
        case _:
            return "unknown"
