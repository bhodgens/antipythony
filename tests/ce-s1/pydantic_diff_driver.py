# tier1-pydiff differential driver: the pydantic band's 110 members.
# Same source (pydantic_diff.c) runs under BOTH runtimes:
#   antipythony: antipythony build-ext + `antipythony run`
#   CPython:     cc -DAPX_CPYTHON_BUILD + python3.14 with this file
# stdout+stderr+rc must byte-match; divergences are recorded findings.
import pydantic_diff as pd

pd.t_mapping()
pd.t_number()
pd.t_sets()
pd.t_complex()
pd.t_unicode()
pd.t_bytes()
pd.t_module()
pd.t_import()
pd.t_errors()
pd.t_types()
pd.t_misc()
pd.t_exc()
pd.t_writer()
print("DIFF_DONE")
