import io
import os
import shutil
import tarfile
import zipfile
from pathlib import Path

import pandas as pd
import PIL.Image
from django.http import Http404, HttpResponse, JsonResponse
from PIL import Image

from docs import imaging

MEDIA = "/srv/media"


# ---- readers that open a client path ---------------------------------------------------------------------------


def preview(request):
    img = Image.open(os.path.join(MEDIA, request.GET["path"]))  # caught
    return HttpResponse(str(img.size))


def preview_module(request):
    img = PIL.Image.open(Path(MEDIA) / request.GET["path"])  # caught
    return HttpResponse(str(img.size))


def table(request):
    frame = pd.read_csv(os.path.join(MEDIA, request.GET["path"]))  # caught
    return HttpResponse(frame.to_html())


def sheet(request):
    frame = pd.read_excel(io=os.path.join(MEDIA, request.GET["path"]))  # caught
    return HttpResponse(frame.to_html())


def archive(request):
    with zipfile.ZipFile(os.path.join(MEDIA, request.POST["archive"])) as zf:  # caught
        return JsonResponse({"names": zf.namelist()})


def tarball(request):
    with tarfile.open(os.path.join(MEDIA, request.POST["archive"])) as tf:  # caught
        return JsonResponse({"names": tf.getnames()})


def tree(request):
    found = []
    for root, _dirs, files in os.walk(os.path.join(MEDIA, request.GET["dir"])):  # caught
        found.extend(os.path.join(root, f) for f in files)
    return JsonResponse({"files": found})


def listing(request):
    names = [p.name for p in (Path(MEDIA) / request.GET["dir"]).iterdir()]  # caught
    return JsonResponse({"names": names})


def matches(request):
    hits = [p.name for p in (Path(MEDIA) / request.GET["dir"]).glob("*.csv")]  # caught
    return JsonResponse({"names": hits})


def bundle(request):
    shutil.make_archive("/tmp/bundle", "zip", root_dir=os.path.join(MEDIA, request.GET["dir"]))  # caught
    return HttpResponse("ok")


# ---- existence oracles: the weaker message, unless a full sink takes the same value ---------------------------


def exists(request):
    return JsonResponse({"ok": os.path.exists(os.path.join(MEDIA, request.GET["n"]))})  # caught


def size(request):
    return JsonResponse({"size": (Path(MEDIA) / request.GET["n"]).stat().st_size})  # caught


def exists_then_open(request):
    path = os.path.join(MEDIA, request.GET["n"])
    if not os.path.isfile(path):
        raise Http404
    return HttpResponse(open(path, "rb"))  # caught


def exists_named(request):
    name = os.path.basename(request.GET["n"])
    return JsonResponse({"ok": os.path.isfile(os.path.join(MEDIA, name))})


# ---- file-like arguments are content, not paths ----------------------------------------------------------------


def thumb_upload(request):
    img = Image.open(request.FILES["image"])
    return HttpResponse(str(img.size))


def thumb_bytes(request):
    img = Image.open(io.BytesIO(request.FILES["image"].read()))
    return HttpResponse(str(img.size))


def csv_upload(request):
    f = request.FILES["sheet"]
    frame = pd.read_csv(f)
    return HttpResponse(frame.to_html())


def zip_upload(request):
    with zipfile.ZipFile(request.FILES.get("archive")) as zf:
        return JsonResponse({"names": zf.namelist()})


def tar_upload(request):
    with tarfile.open(fileobj=request.FILES["archive"]) as tf:
        return JsonResponse({"names": tf.getnames()})


def json_body(request):
    frame = pd.read_json(io.StringIO(request.body.decode()))
    return HttpResponse(frame.to_html())


def own_image(request):
    return HttpResponse(imaging.Image.open(request.GET["name"]))
