import httpx

from svc import requests


def local_get(url):
    return requests.get(url)


def make_client():
    return httpx.AsyncClient(timeout=5)
