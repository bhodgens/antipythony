# Cites: S-01-1, S-01-7, S-01-14, S-01-19, S-01-20, S-02-17, C-31
# (tier modes invisible; deliberate thrash per SPLIT-02.05 §02.05d, Risk 3).
# Stresses: tier-1 thrash → backoff → DISABLED-pin lifecycle (02.05d gate);
# every phase must stay byte-exact while the site deopts and re-opts.
# Deliberate thrash: `mix` flips its hot binop site between int and str
# every 100 calls — 60.1k calls total so the per-function epoch counter
# crosses the 8-epoch DISABLED pin (threshold 64, `threshold << epoch`
# retry backoff, max shift 6). Every phase's results must stay exact;
# the pin observable is the machine-parseable [TIER1-STATS] pin line
# (asserted by the differential runner under APX_TIER1_STATS=1, never by
# the probe itself — this file's stdout must stay byte-identical across
# tier 0 / adaptive, and to the pinned CPython reference).

def mix(a, b):
    return a + b

acc = 0
for i in range(60100):
    if (i // 100) % 2 == 0:
        acc = acc + mix(i, i)
    else:
        acc = acc + len(mix("x", "y"))

print(acc)

# Steady-state int loop AFTER the pin: the (now DISABLED) function keeps
# producing exact values through the boxed twin (pon's entry-pointer
# restore analogue).
post = 0
for i in range(5000):
    post = post + mix(i, 1)
print(post)
