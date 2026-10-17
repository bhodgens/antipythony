# Cites: S-05-4 (row warnings); S-06-10 (record mode: no real-path anchors)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import warnings
names = ["warn", "warn_explicit", "filterwarnings", "simplefilter",
         "resetwarnings", "catch_warnings", "formatwarning", "showwarning",
         "deprecated", "filters", "WarningMessage", "defaultaction",
         "onceregistry"]
print(len(names), all(hasattr(warnings, n) for n in names))
print(repr(warnings.formatwarning("msg", UserWarning, "fake_file.py", 42,
                                  "x = 1")))
print(repr(warnings.formatwarning("m2", DeprecationWarning, "f.py", 1, None)))
print(warnings.defaultaction, type(warnings.filters) is list,
      isinstance(warnings.onceregistry, dict),
      isinstance(warnings.WarningMessage("m", UserWarning, "f", 1, None, None),
                 warnings.WarningMessage))
with warnings.catch_warnings(record=True) as wlist:
    warnings.simplefilter("always")
    warnings.warn("one", UserWarning)
    warnings.warn("two", DeprecationWarning)
print(len(wlist), [str(x.message) for x in wlist],
      [x.category.__name__ for x in wlist])
with warnings.catch_warnings(record=True) as w2:
    warnings.simplefilter("ignore")
    warnings.warn("never")
print(len(w2))
warnings.warn_explicit("we", UserWarning, "nofile", 7)
warnings.resetwarnings()
print("reset-ok", hasattr(warnings, "deprecated"))
