import requests
from django.conf import settings
from django.http import HttpResponse

API = "https://api.example.com"
API_ROOT = "https://api.example.com/"
ARN = "arn:aws:elasticloadbalancing:us-east-1:1:x"


def settings_host(request):
    # The host comes from settings; the client only picks the path.
    return HttpResponse(requests.get(f"https://{settings.API_HOST}/items/{request.GET['id']}", timeout=5).content)


def fixed_host(request):
    return HttpResponse(requests.get(f"https://api.example.com/items/{request.GET['id']}", timeout=5).content)


def constant_host(request):
    return HttpResponse(requests.get(f"{API}/items/{request.GET['id']}", timeout=5).content)


def constant_without_slash(request):
    # `?path=@evil.com` makes evil.com the host: API has no `/` after it.
    return HttpResponse(requests.get(API + request.GET["path"], timeout=5).content)  # caught


def constant_with_slash(request):
    return HttpResponse(requests.get(API_ROOT + request.GET["path"], timeout=5).content)


def subdomain(request):
    return HttpResponse(requests.get(f"https://{request.GET['tenant']}.example.com/", timeout=5).content)  # caught


def server_region(request):
    kid = request.headers.get("x-kid")
    region = ARN.split(":")[3]
    return HttpResponse(requests.get(f"https://keys.{region}.amazonaws.com/{kid}", timeout=5).content)


def format_path(request):
    return HttpResponse(requests.get("https://{}/items/{}".format(settings.API_HOST, request.GET["id"])).content)


def format_host(request):
    return HttpResponse(requests.get("https://{}/items".format(request.GET["host"])).content)  # caught
