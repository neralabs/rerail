import os

from django.http import FileResponse
from django.views import View

MEDIA = "/srv/media"


def serve(request, relpath):
    return FileResponse(open(os.path.join(MEDIA, relpath), "rb"))  # caught


def page(request, name):
    return FileResponse(open(os.path.join(MEDIA, f"{name}.html"), "rb"))


def page_raw(request, name):
    return FileResponse(open(os.path.join(MEDIA, name), "rb"))  # caught


def theme(request, name):
    return FileResponse(open(os.path.join(MEDIA, "themes", "theme-" + name + ".css"), "rb"))


def export(request, export_id):
    return FileResponse(open(os.path.join(MEDIA, f"{export_id}"), "rb"))


def legacy(request, relpath):
    return FileResponse(open(os.path.join(MEDIA, f"{relpath}.html"), "rb"))  # caught


class DownloadView(View):
    def get(self, request, *args, **kwargs):
        return FileResponse(open(os.path.join(MEDIA, self.kwargs["name"]), "rb"))  # caught


def download_q(request):
    name = request.GET["name"]
    if ".." in name:
        raise ValueError(name)
    return FileResponse(open(os.path.join(MEDIA, name), "rb"))  # caught
