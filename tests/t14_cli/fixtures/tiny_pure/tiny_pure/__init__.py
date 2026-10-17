# T-14 fixture package (tiny-pure). Pure Python, no imports beyond stdlib
# allow-list — spec 05 S-05-1..S-05-3 (purity). Imported by scaffolded app
# programs in add/run tests once the CLI exists.


def greet(name):
    return "hello, " + name
