# antipythany v0 stub: the ContextVar surface _py_warnings uses.
# Real context-isolation semantics are a documented v0 gap — a single global
# slot backs every ContextVar.


class ContextVar:
    def __init__(self, name, default=None):
        self._name = name
        self._default = default
        self._value = default
        self._set = False

    @property
    def name(self):
        return self._name

    def get(self, default=None):
        if self._set:
            return self._value
        if default is not None:
            return default
        return self._default

    def set(self, value):
        token = (self._value, self._set)
        self._value = value
        self._set = True
        return token

    def reset(self, token):
        self._value, self._set = token
