import httpx
import requests
from django.http import HttpResponse
from requests.adapters import HTTPAdapter


class GuardedHTTPAdapter(HTTPAdapter):
    """Refuses private, loopback and link-local addresses when it connects."""


class SafeTransport(httpx.HTTPTransport):
    """Resolves the host and refuses private addresses before connecting."""


def guarded_session(request):
    session = requests.Session()
    session.mount("http://", GuardedHTTPAdapter())
    session.mount("https://", GuardedHTTPAdapter())
    return HttpResponse(session.get(request.GET["url"], timeout=5).content)


def retrying_session(request):
    # A library adapter with retries guards nothing.
    session = requests.Session()
    session.mount("https://", HTTPAdapter(max_retries=3))
    return HttpResponse(session.get(request.GET["url"], timeout=5).content)  # caught


def guarded_httpx(request):
    with httpx.Client(transport=SafeTransport()) as client:
        return HttpResponse(client.get(request.GET["url"]).content)


def retrying_httpx(request):
    with httpx.Client(transport=httpx.HTTPTransport(retries=3)) as client:
        return HttpResponse(client.get(request.GET["url"]).content)  # caught


class Fetcher:
    def __init__(self):
        self.session = requests.Session()
        self.session.mount("https://", GuardedHTTPAdapter())

    def fetch(self, url):
        return self.session.get(url, timeout=5)


class PlainFetcher:
    def __init__(self):
        self.session = requests.Session()

    def fetch(self, url):
        return self.session.get(url, timeout=5)  # caught


def preview(request):
    Fetcher().fetch(request.GET["url"])
    PlainFetcher().fetch(request.GET["url"])
    return HttpResponse()


def build_transport(retries):
    transport = SafeTransport(retries=retries)
    return transport


def guarded_factory(request):
    with httpx.Client(transport=build_transport(2)) as client:
        return HttpResponse(client.get(request.GET["url"]).content)
