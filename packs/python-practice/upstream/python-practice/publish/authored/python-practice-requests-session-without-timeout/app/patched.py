import functools
from functools import partial as bind

import requests

from .net import make_session


def with_default_timeout(fn, seconds=10):
    @functools.wraps(fn)
    def call(*args, **kwargs):
        kwargs.setdefault("timeout", seconds)
        return fn(*args, **kwargs)

    return call


def timed_request(method, url, **kwargs):
    kwargs.setdefault("timeout", 10)
    return requests.request(method, url, **kwargs)


# ---- the Session's own method replaced by one that sets a timeout: left alone ------------------------------------


class PartialClient:
    def __init__(self):
        self.session = requests.Session()
        self.session.request = functools.partial(self.session.request, timeout=10)

    def item(self, pk):
        return self.session.get(f"/items/{pk}")

    def create(self, data):
        return self.session.post("/items", json=data)


class PartialSubClient(PartialClient):
    def remove(self, pk):
        return self.session.delete(f"/items/{pk}")


def local_partial(url):
    s = requests.Session()
    s.request = functools.partial(s.request, timeout=(3.05, 27))
    return s.get(url)


def aliased_partial(url):
    s = make_session()
    s.request = bind(s.request, timeout=5)
    return s.head(url)


def wrapper_factory(url):
    s = requests.Session()
    s.request = with_default_timeout(s.request)
    return s.put(url, data=b"x")


def lambda_wrapper(url):
    s = requests.Session()
    original = s.request
    s.request = lambda method, u, **kw: original(method, u, timeout=10, **kw)
    return s.options(url)


def def_replacement(url):
    s = requests.Session()
    s.request = timed_request
    return s.patch(url, json={})


def send_replaced(url):
    s = requests.Session()
    s.send = functools.partial(s.send, timeout=10)
    return s.get(url)


def verb_replaced(url):
    s = requests.Session()
    s.get = functools.partial(s.get, timeout=10)
    return s.get(url)


# ---- replaced without a timeout, or on another Session, or another verb: reported ---------------------------------


def partial_without_timeout(url):
    s = requests.Session()
    s.request = functools.partial(s.request, verify=False)
    return s.get(url)  # caught


def partial_timeout_none(url):
    s = requests.Session()
    s.request = functools.partial(s.request, timeout=None)
    return s.get(url)  # caught


def other_session_patched(url):
    s = requests.Session()
    t = requests.Session()
    t.request = functools.partial(t.request, timeout=10)
    return s.get(url)  # caught


def other_verb_replaced(url):
    s = requests.Session()
    s.get = functools.partial(s.get, timeout=10)
    return s.post(url, json={})  # caught


class HeaderOnlyClient:
    def __init__(self):
        self.session = requests.Session()
        self.session.headers = {"Accept": "application/json"}

    def item(self, pk):
        return self.session.get(f"/items/{pk}")  # caught


# ---- closures: the enclosing function's Session --------------------------------------------------------------------


def in_a_lambda(urls):
    s = requests.Session()
    return list(map(lambda u: s.get(u), urls))  # caught


def in_a_nested_def(urls):
    s = requests.Session()

    def one(u):
        return s.post(u, json={})  # caught

    return [one(u) for u in urls]


def in_a_lambda_with_timeout(urls):
    s = requests.Session()
    return list(map(lambda u: s.get(u, timeout=5), urls))


def closure_rebound(urls):
    s = requests.Session()
    s = None

    def one(u):
        return s.get(u)

    return [one(u) for u in urls]
