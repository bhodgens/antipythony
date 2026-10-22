# antipythany deviation shim — see package __init__.py.
class TranslatorMixin:
    pass


class HTMLTranslator(TranslatorMixin):
    def css_to_xpath(self, css, prefix="descendant-or-self::"):
        raise NotImplementedError("css selectors require lxml")

    def __call__(self, css, prefix="descendant-or-self::"):
        raise NotImplementedError("css selectors require lxml")


GenericTranslator = HTMLTranslator
