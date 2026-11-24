"""Repro 'object' object is not callable during lazy-import chain.

Package Q: __getattr__ lazy-imports Q.main; Q.main does
`from Q import thing` (re-entrant from-import through Q.__getattr__ ->
Q.sub) AND calls object(). If the nested from-import poisons the builtin
`object` binding in Q.main's globals, we reproduce the pydantic failure.
"""
import Q

print("Q-MAIN-OBJECT:", Q.OUTPUT)
