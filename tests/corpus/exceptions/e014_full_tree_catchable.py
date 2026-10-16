# Cites: S-03-32, S-03-34
# Every direct subclass of Exception in the live 3.14 tree is a real,
# catchable class: derive the list from Exception.__subclasses__() itself
# (deterministic on builtins-only startup), raise one instance of each under a
# tuple target of all of them, and check identity of the catch. Group types
# need (message, exceptions) args; the rest take one arg.
subs = sorted(Exception.__subclasses__(), key=lambda t: t.__name__)
print(len(subs))
GROUPY = {"ExceptionGroup", "BaseExceptionGroup"}
targets = tuple(subs)
ok = []
for t in subs:
    try:
        if t.__name__ in GROUPY:
            raise t("m", [ValueError("x")])
        else:
            raise t("probe")
    except targets as e:
        ok.append(type(e) is t)
print(all(ok), len(ok))
print([t.__name__ for t in subs])
