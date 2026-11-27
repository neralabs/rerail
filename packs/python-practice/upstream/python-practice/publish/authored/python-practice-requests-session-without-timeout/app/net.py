import logging

import requests
from requests.adapters import HTTPAdapter

logger = logging.getLogger(__name__)

# A module-wide session, shared by the importers below.
session = requests.Session()


class TimeoutHTTPAdapter(HTTPAdapter):
    def __init__(self, *args, timeout=10, **kwargs):
        self.timeout = timeout
        super().__init__(*args, **kwargs)

    def send(self, request, **kwargs):
        if kwargs.get("timeout") is None:
            kwargs["timeout"] = self.timeout
        return super().send(request, **kwargs)


class RetryAdapter(HTTPAdapter):
    def send(self, request, **kwargs):
        logger.debug("sending %s", request.url)
        return super().send(request, **kwargs)


class TimeoutSession(requests.Session):
    def request(self, method, url, **kwargs):
        kwargs.setdefault("timeout", 10)
        return super().request(method, url, **kwargs)


class LoggingSession(requests.Session):
    def request(self, method, url, **kwargs):
        logger.info("%s %s", method, url)
        return super().request(method, url, **kwargs)


class AttributeTimeoutSession(requests.Session):
    def __init__(self):
        super().__init__()
        self.timeout = 10  # requests never reads this attribute


def make_session():
    return requests.Session()


def make_mounted_session():
    s = requests.Session()
    s.mount("https://", TimeoutHTTPAdapter(timeout=5))
    return s


def ping_at_import():
    return session.head("https://example.com/health")  # caught
