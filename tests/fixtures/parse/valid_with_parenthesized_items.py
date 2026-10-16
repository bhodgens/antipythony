with (
    open("a") as a,
    open("b") as b,
    ctx() as c,
):
    result = a.read() + b.read()
