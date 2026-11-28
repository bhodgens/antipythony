# Cites: S-03-32, S-03-34
# Every direct subclass of Exception in the live 3.14 tree is a real,
# catchable class: derive the list from Exception.__subclasses__() itself
# (deterministic once re is imported — see below), raise one instance of each
# under a tuple target of all of them, and check identity of the catch. Group
# types need (message, exceptions) args; the rest take one arg.
# `import re` is load-bearing (issue #29): the pin counts PatternError (22
# subclasses); re.PatternError exists in the tree only after re is imported.
# The recording host's Homebrew sitecustomize pre-imports re, CI's
# python-build-standalone interpreter does not — the explicit import makes
# the 22-count pin byte-reproducible on any CPython 3.14 (see e002).
import re  # noqa: F401 — ensures re.PatternError is in Exception.__subclasses__()
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
