import socket
import time
import urllib.request

import httpx
import requests


def fetch(url):
    return requests.get(url, timeout=5)


def post_json(url, body):
    return httpx.post(url, json=body, timeout=5)


def download(url):
    with urllib.request.urlopen(url, timeout=5) as r:
        return r.read()


def resolve(host):
    return socket.gethostbyname(host)


def retry_fetch(url):
    for _ in range(3):
        try:
            return fetch(url)
        except requests.RequestException:
            time.sleep(1)
    return None


def build_url(base, path):
    return f"{base}/{path}"


def later(url):
    return lambda: requests.get(url, timeout=5)


def deferred(url):
    def run():
        return fetch(url)

    return run


async def afetch(url):
    async with httpx.AsyncClient() as client:
        return await client.get(url)


def make_coroutine(url):
    return afetch(url)
