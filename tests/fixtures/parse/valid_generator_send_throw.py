def echo():
    while True:
        try:
            received = yield "ready"
        except ValueError:
            yield "caught"
        else:
            yield received
