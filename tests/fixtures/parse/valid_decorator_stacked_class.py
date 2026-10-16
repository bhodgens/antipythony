def annotate(cls):
    cls.annotated = True
    return cls

def register(fn):
    fn.registered = True
    return fn

@register
@annotate
class Widget:
    @staticmethod
    def make():
        return Widget()

@register
def reset():
    pass
