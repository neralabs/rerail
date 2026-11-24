import urllib.request
from urllib.parse import urlparse

from bookmarks import http_client

ALLOWED_FEED_HOSTS = {"feeds.example.com"}


def load_website(url):
    response = http_client.fetch(url)
    return response.text


def probe_url(url):
    return http_client.head(url).status_code < 400


def read_feed(feed_url):
    with urllib.request.urlopen(feed_url, timeout=10) as fh:  # caught
        return fh.read()


def get_validated_feed_url(url):
    if urlparse(url).hostname not in ALLOWED_FEED_HOSTS:
        raise ValueError("feed host not allowed")
    return url


def read_item(item_id):
    return http_client.fetch_item(item_id).json()
