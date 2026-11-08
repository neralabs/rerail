import requests
from requests import post


def fetch(url):
    return requests.get(url)


def fetch_forever(url):
    return requests.get(url, timeout=None)


def fetch_bounded(url):
    return requests.get(url, timeout=5)


def upload(url, data):
    return post(url, data=data)


def upload_bounded(url, data):
    return post(url, data=data, timeout=(3.05, 27))
