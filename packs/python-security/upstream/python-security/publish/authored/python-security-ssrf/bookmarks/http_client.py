import requests
from django.conf import settings

session = requests.Session()


def fetch(url, timeout=10):
    return requests.get(url, timeout=timeout)  # caught


def head(url):
    return session.head(url, allow_redirects=True)  # caught


def fetch_item(item_id):
    # The host is fixed; the client picks only a path segment.
    return requests.get(f"{settings.CATALOG_API}/items/{item_id}", timeout=5)


def search(term):
    return requests.get("https://api.example.com/search", params={"q": term}, timeout=5)
