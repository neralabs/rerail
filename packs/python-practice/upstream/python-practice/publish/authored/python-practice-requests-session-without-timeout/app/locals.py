import httpx
import requests
from requests import Session
from requests.adapters import HTTPAdapter

from .net import (
    AttributeTimeoutSession,
    LoggingSession,
    RetryAdapter,
    TimeoutHTTPAdapter,
    TimeoutSession,
    make_mounted_session,
    make_session,
)

TIMEOUT = (3.05, 27)


def fetch(url):
    s = requests.Session()
    return s.get(url)  # caught


def post_it(url, data):
    with requests.Session() as s:
        return s.post(url, data=data)  # caught


def lower_case_factory(url):
    s = requests.session()
    s.head(url)  # caught


def imported_class(url):
    s = Session()
    return s.request("GET", url)  # caught


def chained(url):
    return requests.Session().get(url)  # caught


def through_a_name(url):
    s = requests.Session()
    other = s
    return other.options(url)  # caught


def none_timeout(url):
    s = requests.Session()
    return s.get(url, timeout=None)  # caught


def factory(url):
    return make_session().get(url)  # caught


def plain_adapter(url):
    s = requests.Session()
    s.mount("https://", HTTPAdapter(max_retries=3))
    return s.put(url, data=b"x")  # caught


def adapter_without_timeout(url):
    s = requests.Session()
    s.mount("https://", RetryAdapter())
    return s.delete(url)  # caught


def logging_session(url):
    s = LoggingSession()
    return s.get(url)  # caught


def attribute_timeout_session(url):
    s = AttributeTimeoutSession()
    return s.patch(url, json={})  # caught


# ---- left alone --------------------------------------------------------------------------------------------------


def with_timeout(url):
    s = requests.Session()
    return s.get(url, timeout=TIMEOUT)


def streaming(url):
    with requests.Session() as s:
        return s.get(url, stream=True, timeout=(5, None))


def passthrough(url, **kwargs):
    s = requests.Session()
    return s.get(url, **kwargs)


def positional_timeout(url):
    s = requests.Session()
    return s.request("GET", url, None, None, None, None, None, None, 5)


def factory_mounted(url):
    return make_mounted_session().get(url)


def mounted_local(url):
    s = requests.Session()
    adapter = TimeoutHTTPAdapter(timeout=5)
    s.mount("https://", adapter)
    s.mount("http://", adapter)
    return s.get(url)


def timeout_session(url):
    s = TimeoutSession()
    return s.get(url)


def rebound(url):
    s = requests.Session()
    s = TimeoutSession()
    return s.get(url)


def httpx_client(url):
    c = httpx.Client()
    return c.get(url)


def module_function(url):
    return requests.get(url, timeout=5)


def parameter(session, url):
    return session.get(url)


def not_an_http_call(url):
    s = requests.Session()
    s.headers.update({"Accept": "application/json"})
    return s.cookies.get("sid")


def sqlalchemy_session(engine, pk):
    from sqlalchemy.orm import Session

    with Session(engine) as s:
        return s.get(dict, pk)


async def aiohttp_session(url):
    import aiohttp

    async with aiohttp.ClientSession() as s:
        return await s.get(url)
