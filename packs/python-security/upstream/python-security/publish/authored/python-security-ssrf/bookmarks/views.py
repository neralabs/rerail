import urllib.request
from urllib.parse import urlparse

import requests
import validators
from django.conf import settings
from django.http import HttpResponse, JsonResponse
from rest_framework.decorators import api_view
from rest_framework.response import Response
from rest_framework.views import APIView

from bookmarks import services
from bookmarks.models import Bookmark
from bookmarks.security import is_allowed_host, validate_url

ALLOWED_HOSTS = {"hooks.example.com"}


# ---- reported ----------------------------------------------------------------------------------------------------


def preview(request):
    return HttpResponse(requests.get(request.GET["url"], timeout=5).text)  # caught


def ping(request):
    url = request.POST.get("url")
    resp = requests.post(url, data={"ping": "1"}, timeout=5)  # caught
    return HttpResponse(resp.status_code)


def proxy(request):
    method = request.GET.get("method", "GET")
    resp = requests.request(method, request.GET["target"], timeout=5)  # caught
    return HttpResponse(resp.content)


@api_view(["POST"])
def check_bookmark(request):
    url = request.data["url"]
    return Response({"title": services.load_website(url)})


@api_view(["POST"])
def reachable(request):
    return Response({"ok": services.probe_url(request.data.get("url"))})


def status(request):
    host = request.GET["host"]
    return JsonResponse(requests.get(f"https://{host}/status", timeout=5).json())  # caught


def mirror(request):
    base = request.GET["base"]
    return JsonResponse(requests.get(f"{base}/api/v1/ping", timeout=5).json())  # caught


def concat(request):
    resp = requests.get("http://" + request.GET["host"] + "/health", timeout=5)  # caught
    return HttpResponse(resp.status_code)


def percent(request):
    resp = requests.get("https://%s/health" % request.GET["host"], timeout=5)  # caught
    return HttpResponse(resp.status_code)


def percent_tuple(request):
    scheme, host = "https", request.GET["host"]
    resp = requests.get("%s://%s/health" % (scheme, host), timeout=5)  # caught
    return HttpResponse(resp.status_code)


def formatted(request):
    resp = requests.get("https://{}/health".format(request.GET["host"]), timeout=5)  # caught
    return HttpResponse(resp.status_code)


def with_session(request):
    target = request.POST["webhook"]
    with requests.Session() as s:
        s.post(target, json={"event": "test"}, timeout=5)  # caught
    return HttpResponse("sent")


def session_var(request):
    s = requests.Session()
    s.headers["User-Agent"] = "bot"
    return HttpResponse(s.get(url=request.GET["u"], timeout=5).text)  # caught


def format_only(request):
    url = request.GET["url"]
    if not validators.url(url):
        return HttpResponse(status=400)
    return HttpResponse(requests.get(url, timeout=5).text)  # caught


def feed(request):
    return HttpResponse(services.read_feed(request.GET["feed"]))


def opener(request):
    req = urllib.request.Request(request.GET["url"], headers={"User-Agent": "bot"})  # caught
    return HttpResponse(urllib.request.urlopen(req).read())


class FetchView(APIView):
    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.http = requests.Session()

    def post(self, request):
        url = self.request.data.get("url")
        return Response({"body": self.http.get(url, timeout=5).text})  # caught


# ---- left alone --------------------------------------------------------------------------------------------------


def item(request):
    item_id = request.GET["id"]
    return JsonResponse(requests.get(f"{settings.CATALOG_API}/items/{item_id}", timeout=5).json())


def item_fixed(request):
    item_id = request.GET["id"]
    return JsonResponse(requests.get(f"https://api.example.com/items/{item_id}", timeout=5).json())


def search(request):
    params = {"q": request.GET.get("q", "")}
    return JsonResponse(requests.get("https://api.example.com/search", params=params, timeout=5).json())


def item_concat(request):
    resp = requests.get("https://api.example.com/users/" + request.GET["user"], timeout=5)
    return HttpResponse(resp.status_code)


def item_base_concat(request):
    resp = requests.get(settings.CATALOG_API + request.GET["path"], timeout=5)
    return HttpResponse(resp.status_code)


def item_percent(request):
    resp = requests.get("https://%s/items/%s" % (settings.CATALOG_HOST, request.GET["id"]), timeout=5)
    return HttpResponse(resp.status_code)


def item_format(request):
    resp = requests.get("https://api.example.com/items/{}".format(request.GET["id"]), timeout=5)
    return HttpResponse(resp.status_code)


def query_after_host(request):
    resp = requests.get(f"https://api.example.com/redirect?to=https://{request.GET['host']}", timeout=5)
    return HttpResponse(resp.status_code)


def notify(request):
    requests.post(settings.WEBHOOK_URL, json={"url": request.POST["url"]}, timeout=5)
    return HttpResponse("ok")


def checked(request):
    url = request.GET["url"]
    if not validate_url(url):
        return HttpResponse(status=400)
    return HttpResponse(requests.get(url, timeout=5).text)


def allow_listed(request):
    url = request.GET["url"]
    parsed = urlparse(url)
    if parsed.hostname not in ALLOWED_HOSTS:
        return HttpResponse(status=400)
    return HttpResponse(requests.get(url, timeout=5).text)


def allow_helper(request):
    url = request.GET["url"]
    if not is_allowed_host(urlparse(url).hostname):
        return HttpResponse(status=400)
    return HttpResponse(requests.get(url, timeout=5).text)


def validated_feed(request):
    url = services.get_validated_feed_url(request.GET["feed"])
    return HttpResponse(requests.get(url, timeout=5).text)


def stored(request):
    bookmark = Bookmark.objects.get(pk=int(request.GET["id"]))
    return HttpResponse(requests.get(bookmark.url, timeout=5).text)


def session_lookup(request):
    return HttpResponse(request.session.get(request.GET["key"]))


def from_settings(request):
    return HttpResponse(requests.get(settings.STATUS_URL, timeout=5).text)


def item_service(request):
    return JsonResponse(services.read_item(request.GET["id"]))
