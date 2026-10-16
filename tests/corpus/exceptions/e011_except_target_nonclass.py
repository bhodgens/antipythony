# Cites: S-03-34
# Except target may be a non-class expression or a type outside the
# BaseException tree (object): both fail at catch time with the pinned
# TypeError — the clause is never silently skipped.
def t(label, fn):
    try:
        fn()
        print(label, "-> caught")
    except TypeError as e:
        print(label, "-> TypeError:", e)
def five():
    try:
        raise ValueError()
    except 5:
        print("body")
t("except-int", five)
def obj():
    try:
        raise ValueError()
    except object:
        print("body")
t("except-object", obj)
def strtarget():
    try:
        raise ValueError()
    except "ValueError":
        print("body")
t("except-str", strtarget)
