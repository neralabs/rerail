import functools
import socket
import subprocess
from functools import lru_cache

import backoff
import httpx
import requests
import tenacity
from cachetools import TTLCache, cached
from tenacity import retry as tretry
from tenacity import stop_after_attempt
from tracing import traced

from svc.decorators import in_thread, logged, retrying, submitted, throttled


@tenacity.retry(stop=stop_after_attempt(3))
def fetch_retry(url):
    return requests.get(url, timeout=5)


@tretry
def fetch_alias(url):
    return requests.get(url, timeout=5)


@backoff.on_exception(backoff.expo, OSError, max_tries=3)
def connect(host):
    return socket.create_connection((host, 443), timeout=5)


@lru_cache(maxsize=None)
def git_sha():
    # Memoized with no argument: it blocks once per process.
    return subprocess.check_output(["git", "rev-parse", "HEAD"], text=True)


@functools.cache
def lookup(host):
    return socket.gethostbyname(host)


@cached(cache=TTLCache(maxsize=64, ttl=60))
def rates(base):
    return requests.get(f"https://rates.local/{base}", timeout=5).json()


@functools.lru_cache
@tenacity.retry(stop=stop_after_attempt(3))
def stacked(url):
    return requests.get(url, timeout=5)


@traced
def unknown_wrapped(url):
    # A decorator from outside the run this rule does not know: the call stays open.
    return requests.get(url, timeout=5)


@logged
def slow_get(url):
    return requests.get(url, timeout=5)


@retrying(3)
def flaky(url):
    return httpx.get(url, timeout=5)


@throttled
def compute(x):
    return x * 2


@submitted
def submitted_get(url):
    return requests.get(url, timeout=5)


@in_thread
def threaded_get(url):
    return requests.get(url, timeout=5)


class Client:
    @functools.lru_cache(maxsize=None)
    def version(self):
        return subprocess.check_output(["tool", "--version"], text=True)

    @tenacity.retry(stop=stop_after_attempt(3))
    def get(self, url):
        return requests.get(url, timeout=5)

    async def info(self, url):
        v = self.version()
        return v, self.get(url)  # caught
