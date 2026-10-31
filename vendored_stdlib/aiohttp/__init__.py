"""aiohttp — minimal import-success surface (antipythany vendored).

Third-party async HTTP client used by some scrapy components (remote_control
extension, test data only). The crawl gate never issues HTTP requests, so all
constructors validate and all network operations raise NotImplementedError.
"""
__all__ = ["ClientSession", "ClientTimeout", "TCPConnector", "ClientError",
           "ClientConnectionError", "ClientResponseError"]


class ClientError(Exception):
    pass


class ClientConnectionError(ClientError):
    pass


class ClientResponseError(ClientError):
    def __init__(self, request_info=None, history=None, *, status=None,
                 message="", headers=None):
        super().__init__(status, message, request_info, history)
        self.status = status
        self.message = message
        self.headers = headers


class ClientTimeout:
    def __init__(self, total=300, connect=None, sock_read=None, sock_connect=None):
        self.total = total
        self.connect = connect
        self.sock_read = sock_read
        self.sock_connect = sock_connect


class TCPConnector:
    def __init__(self, *a, limit=100, **kw):
        self.limit = limit

    async def close(self):
        return None


class ClientSession:
    def __init__(self, *a, connector=None, timeout=None, headers=None, **kw):
        self.connector = connector
        self.timeout = timeout
        self.headers = headers or {}

    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        await self.close()
        return False

    async def close(self):
        return None

    def _request(self, method, url, **kw):
        raise NotImplementedError("aiohttp v0 shim: network unimplemented")

    get = lambda self, url, **kw: self._request("GET", url, **kw)
    post = lambda self, url, **kw: self._request("POST", url, **kw)
    put = lambda self, url, **kw: self._request("PUT", url, **kw)
    delete = lambda self, url, **kw: self._request("DELETE", url, **kw)
