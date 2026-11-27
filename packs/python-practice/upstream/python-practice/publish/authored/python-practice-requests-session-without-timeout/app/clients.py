import requests

from . import net
from .net import TimeoutSession, session


class ApiClient:
    def __init__(self, base):
        self.base = base
        self.session = requests.Session()
        self.session.headers.update({"Accept": "application/json"})

    def get_item(self, pk):
        return self.session.get(f"{self.base}/items/{pk}")  # caught

    def create(self, data):
        return self.session.post(f"{self.base}/items", json=data, timeout=10)


class SubClient(ApiClient):
    def delete_item(self, pk):
        return self.session.delete(f"{self.base}/items/{pk}")  # caught


class LazyClient:
    def __init__(self):
        self._session = None

    def _new_session(self):
        session = requests.Session()
        return session

    def _ensure(self):
        if self._session is None:
            self._session = self._new_session()

    def send(self, url, payload):
        self._ensure()
        return self._session.patch(url, json=payload)  # caught


class ClassAttributeClient:
    session = requests.Session()

    def ping(self, url):
        return self.session.options(url)  # caught


class SafeClient:
    def __init__(self):
        self.session = TimeoutSession()

    def fetch(self, url):
        return self.session.get(url)


class InjectedClient:
    def __init__(self, session):
        self.session = session

    def fetch(self, url):
        return self.session.get(url)


def from_global(url):
    return session.get(url)  # caught


def from_module(url):
    return net.session.post(url, json={})  # caught


def unrelated_session(db):
    return db.session.get(1)
