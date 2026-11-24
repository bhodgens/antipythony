# This body runs while Q is partially initialized. The re-entrant from-import
# resolves Q.thing through Q.__getattr__ (Q.sub). Then object() must still be
# the builtin TYPE.
from Q import thing

OUTPUT = "THING:" + thing + ":OBJECT:" + type(object()).__name__
