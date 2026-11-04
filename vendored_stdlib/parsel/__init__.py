# antipythony deviation shim (DEVIATIONS.toml): real parsel depends on
# lxml (C extension; docs/compat-contract.md C-07). The T1 scrapy chain
# imports parsel only for type surfaces (TextResponse.selector is lazily
# created and the gate spider parses with `re`). v0 shim: name surface with
# NotAllowed-implementation selectors.
from .selector import Selector, SelectorList

__all__ = ["Selector", "SelectorList"]
