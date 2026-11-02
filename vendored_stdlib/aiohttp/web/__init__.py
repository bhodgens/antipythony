"""aiohttp.web — v0 import-success shim (antipythany).

remote_control extension imports this at module level; never enabled in the
crawl gate. Application/RouteTableDef constructors validate only; run() raises
NotImplementedError lazily.
"""


class Application:
    def __init__(self, *args, **kwargs):
        self.router = Router()

    def add_routes(self, routes):
        pass

    def make_handler(self, *a, **k):
        raise NotImplementedError("aiohttp.web server ops not implemented (v0)")


class Router:
    def __init__(self):
        self.routes = []

    def add_get(self, *a, **k):
        pass

    def add_post(self, *a, **k):
        pass


class RouteTableDef:
    def __init__(self):
        self._routes = []

    def get(self, path):
        def deco(fn):
            self._routes.append((path, fn))
            return fn
        return deco

    def post(self, path):
        def deco(fn):
            self._routes.append((path, fn))
            return fn
        return deco


def run_app(*a, **k):
    raise NotImplementedError("aiohttp.web server ops not implemented (v0)")


__all__ = ["Application", "Router", "RouteTableDef", "run_app"]
