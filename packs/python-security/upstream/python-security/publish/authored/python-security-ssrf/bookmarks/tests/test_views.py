import requests

from bookmarks import http_client


def test_preview(request):
    assert requests.get(request.GET["url"], timeout=5).ok


def test_fetch(request):
    assert http_client.fetch(request.GET["url"]).ok
