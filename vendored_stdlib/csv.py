"""antipythony deviation shim — csv (v0: the file:// gate's spider output
writers only need reader/writer over line iterables)."""

import re


def reader(csvfile, dialect="excel", **fmtparams):
    return _Reader(csvfile)


class _Reader:
    def __init__(self, csvfile):
        self.csvfile = csvfile

    def __iter__(self):
        return iter(self._rows())

    def _rows(self):
        for line in self.csvfile:
            if isinstance(line, bytes):
                line = line.decode("utf-8")
            line = line.rstrip("\r\n")
            if line:
                yield _parse_line(line)

    def __next__(self):
        for row in self._rows():
            return row
        raise StopIteration


def _parse_line(line):
    # v0: naive split honoring simple double-quote escaping.
    if '"' not in line:
        return line.split(",")
    fields = []
    cur = []
    in_quotes = False
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == '"':
            if in_quotes and i + 1 < len(line) and line[i + 1] == '"':
                cur.append('"')
                i += 1
            else:
                in_quotes = not in_quotes
        elif ch == "," and not in_quotes:
            fields.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
        i += 1
    fields.append("".join(cur))
    return fields


class _Writer:
    def __init__(self, csvfile):
        self.csvfile = csvfile

    def writerow(self, row):
        def esc(v):
            s = v if isinstance(v, str) else str(v)
            if "," in s or '"' in s or "\n" in s:
                s = '"' + s.replace('"', '""') + '"'
            return s

        self.csvfile.write(",".join(esc(v) for v in row) + "\r\n")

    def writerows(self, rows):
        for row in rows:
            self.writerow(row)


def writer(csvfile, dialect="excel", **fmtparams):
    return _Writer(csvfile)


def DictReader(f, fieldnames=None, restkey=None, restval=None, *args, **kwargs):
    class _DictReader:
        def __init__(self):
            self._f = f
            self._fn = fieldnames

        def __iter__(self):
            header = None
            for row in reader(f):
                if header is None:
                    header = row
                    continue
                yield dict(zip(header or [], row))

    return _DictReader()


def DictWriter(f, fieldnames, restval="", extrasaction="raise",
               dialect="excel", *args, **kwds):
    class _DictWriter:
        def writerow(self, d):
            writer(f).writerow([d.get(k, restval) for k in fieldnames])

        def writerows(self, rows):
            for d in rows:
                self.writerow(d)

    return _DictWriter()


def field_size_limit(*new_limit):
    return 131072


def list_dialects():
    return ["excel", "excel-tab", "unix"]


def get_dialect(name):
    return "excel"


def register_dialect(name, dialect=None, **fmtparams):
    return None


def unregister_dialect(name):
    return None


QUOTE_ALL = 1
QUOTE_MINIMAL = 0
QUOTE_NONNUMERIC = 2
QUOTE_NONE = 3
