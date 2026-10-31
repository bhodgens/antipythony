"""antipythany deviation shim — cryptography.hazmat (v0).

Real package so `import cryptography.hazmat.X` resolves on runtimes that
don't synthesize parent packages for pre-registered sys.modules entries.
Attribute surface lives in the leaf subpackages/modules.
"""
