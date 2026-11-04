"""ftplib — minimal stdlib-compatible surface (antipythony vendored).

FTP client used by scrapy's feedexport only for FTP URLs (test data);
the FTP storages are not exercised in the file:// gate. Provide the class
surface so imports succeed; real network methods raise NotImplementedError.
"""
__all__ = ["FTP", "FTP_TLS", "error_perm", "error_temp", "error_proto", "all_errors"]


class Error(Exception):
    pass


class error_temp(Error):
    pass


class error_perm(Error):
    pass


class error_proto(Error):
    pass


all_errors = (Error, error_temp, error_perm, error_proto, OSError)


class FTP:
    """v0 FTP client: constructor validates args; network ops unimplemented."""

    def __init__(self, host="", user="", passwd="", acct="", timeout=None):
        self.host = host
        self.port = 21
        self.timeout = timeout
        self.sock = None
        self.file = None
        self.welcome = None
        if host:
            self.connect(host)

    def connect(self, host="", port=0, timeout=None, source_address=None):
        raise NotImplementedError("ftplib v0 shim: network unimplemented")

    def login(self, user="anonymous", passwd="", acct=""):
        raise NotImplementedError("ftplib v0 shim: network unimplemented")

    def cwd(self, dirname):
        raise NotImplementedError("ftplib v0 shim: network unimplemented")

    def quit(self):
        return None

    def close(self):
        return None

    def storbinary(self, cmd, fp, blocksize=8192, callback=None, rest=None):
        raise NotImplementedError("ftplib v0 shim: network unimplemented")

    def retrbinary(self, cmd, callback, blocksize=8192, rest=None):
        raise NotImplementedError("ftplib v0 shim: network unimplemented")


class FTP_TLS(FTP):
    """v0 shim: TLS FTP shares the inert surface; auth() unimplemented."""

    def __init__(self, host="", user="", passwd="", acct="", timeout=None,
                 context=None):
        super().__init__(host, user, passwd, acct, timeout)
        self.context = context

    def auth(self):
        raise NotImplementedError("ftplib v0 shim: network unimplemented")

    def prot_p(self):
        raise NotImplementedError("ftplib v0 shim: network unimplemented")
