from urllib.parse import urlparse

import requests
from django.core.validators import URLValidator
from django.http import HttpResponse
from pydantic import HttpUrl, TypeAdapter

from hosts.http import get_client

BLOCKED = {"http://169.254.169.254/"}
HOSTS = {"api.example.com", "cdn.example.com"}


def django_urlvalidator(request):
    url = request.GET["url"]
    validate = URLValidator(schemes=["http", "https"])
    validate(url)
    return HttpResponse(requests.get(url, timeout=5).content)  # caught


def inline_urlvalidator(request):
    url = request.GET["url"]
    URLValidator()(url)
    return HttpResponse(requests.get(url, timeout=5).content)  # caught


def pydantic_typeadapter(request):
    url = request.GET["url"]
    TypeAdapter(HttpUrl).validate_python(url)
    return HttpResponse(requests.get(url, timeout=5).content)  # caught


def blocklist(request):
    url = request.GET["url"]
    if url in BLOCKED:
        raise ValueError(url)
    return HttpResponse(requests.get(url, timeout=5).content)  # caught


def factory_client(request):
    return HttpResponse(get_client().get(request.GET["url"]).content)  # caught


def allowlist_chain(request):
    path = request.GET["path"]
    parsed_url = urlparse(path)
    host = parsed_url.netloc
    if host not in HOSTS:
        return HttpResponse(status=400)
    return HttpResponse(requests.get(path, timeout=5).content)


def is_allowed_url(url):
    return urlparse(url).hostname in HOSTS


def checked_call(request):
    url = request.GET["url"]
    if not is_allowed_url(url):
        return HttpResponse(status=400)
    return HttpResponse(requests.get(url, timeout=5).content)


def check_url(url):
    # Named like a check, but it fetches the URL itself.
    return requests.head(url, timeout=5).ok  # caught


def fetch_after_probe(request):
    url = request.GET["url"]
    if not check_url(url):
        return HttpResponse(status=400)
    return HttpResponse(requests.get(url, timeout=5).content)  # caught


def proxy_then_inspect(request):
    # Checking what came back says nothing about where it was fetched from.
    url = request.GET["url"]
    response = requests.get(url, timeout=5)  # caught
    kind = response.headers.get("content-type", "")
    if kind not in {"image/png", "image/jpeg"}:
        return HttpResponse(status=415)
    return HttpResponse(response.content)
