# Cites: S-01-27 S-02-15
# Pins: falsy set drives branch selection (0, 0.0, '', None).
def t(v):
    if v:
        return 'truthy'
    return 'falsy'
print(t(0), t(1), t(0.0), t(2.5), t(''), t('x'), t(None))
