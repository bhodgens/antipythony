# antipythany deviation shim — see package __init__.py.
class _NoSelector:
    def __init__(self, *args, **kwargs):
        raise NotImplementedError(
            "parsel selectors require lxml (C extension); "
            "not supported in this runtime"
        )

    def xpath(self, *args, **kwargs):
        raise NotImplementedError("parsel xpath requires lxml")

    def css(self, *args, **kwargs):
        raise NotImplementedError("parsel css requires lxml")


Selector = _NoSelector


class SelectorList(list):
    pass
