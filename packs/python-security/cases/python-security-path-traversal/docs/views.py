import os
import shutil
from io import BytesIO
from pathlib import Path
from uuid import uuid4

from django.conf import settings
from django.core.files.storage import FileSystemStorage, default_storage
from django.http import FileResponse, Http404, HttpResponse
from django.utils.text import get_valid_filename
from PIL import Image

from docs import storage
from docs.models import Document

MEDIA = settings.MEDIA_ROOT


# ---- reported ----------------------------------------------------------------------------------------------------


def download(request):
    name = request.GET["name"]
    return FileResponse(open(os.path.join(MEDIA, name), "rb"))  # caught


def delete_export(request):
    os.remove(os.path.join(MEDIA, "exports", request.POST["file"]))  # caught


def drop_folder(request):
    folder = request.POST.get("folder", "")
    shutil.rmtree(MEDIA + "/kb/" + folder)  # caught


def browse(request):
    entries = os.listdir(os.path.join(MEDIA, request.GET.get("dir", "")))  # caught
    return HttpResponse("\n".join(entries))


def upload(request):
    f = request.FILES["file"]
    dest = os.path.join(MEDIA, f.name)
    with open(dest, "wb") as out:  # caught
        for chunk in f.chunks():
            out.write(chunk)


def upload_normalised(request):
    f = request.FILES["file"]
    path = os.path.normpath(os.path.join(MEDIA, f.name.replace("../", "")))
    with open(path, "wb") as out:  # caught
        out.write(f.read())


def read_note(request):
    return HttpResponse((Path(MEDIA) / request.GET["note"]).read_text())  # caught


def prefix_checked(request):
    target = os.path.realpath(os.path.join(MEDIA, request.GET["p"]))
    if not target.startswith(MEDIA):
        raise Http404
    return FileResponse(open(target, "rb"))  # caught


def commonprefix_checked(request):
    target = os.path.abspath(os.path.join(MEDIA, request.GET["p"]))
    if os.path.commonprefix([target, MEDIA]) != MEDIA:
        raise Http404
    return FileResponse(open(target, "rb"))  # caught


def unresolved_relative(request):
    target = Path(MEDIA) / request.GET["p"]
    if not target.is_relative_to(MEDIA):
        raise Http404
    return HttpResponse(target.read_bytes())  # caught


def user_storage(request):
    store = FileSystemStorage(location=os.path.join(MEDIA, request.GET["space"]))  # caught
    return HttpResponse(store.url("index.html"))


def get_doc(request):
    return HttpResponse(storage.read(request.GET["doc"]))


# ---- left alone ----------------------------------------------------------------------------------------------------


def download_basename(request):
    name = os.path.basename(request.GET["name"])
    return FileResponse(open(os.path.join(MEDIA, name), "rb"))


def download_renamed(request):
    name = request.GET["name"]
    name = get_valid_filename(name)
    return FileResponse(open(os.path.join(MEDIA, name), "rb"))


def upload_valid_name(request):
    f = request.FILES["file"]
    with open(os.path.join(MEDIA, get_valid_filename(f.name)), "wb") as out:
        out.write(f.read())


def upload_storage(request):
    f = request.FILES["file"]
    default_storage.save(f.name, f)


def upload_generated(request):
    f = request.FILES["file"]
    with open(os.path.join(MEDIA, uuid4().hex), "wb") as out:
        out.write(f.read())


def thumbnail(request):
    f = request.FILES["image"]
    img = Image.open(BytesIO(f.read()))
    return HttpResponse(str(img.size))


def temp_upload(request):
    f = request.FILES["file"]
    with open(f.temporary_file_path(), "rb") as fh:
        return HttpResponse(fh.read(10))


def export_by_id(request):
    export_id = int(request.GET["id"])
    return FileResponse(open(os.path.join(MEDIA, f"export-{export_id}.csv"), "rb"))


def stored(request):
    doc = Document.objects.get(pk=request.GET["id"])
    return FileResponse(open(doc.path, "rb"))


def contained(request):
    base = os.path.realpath(MEDIA)
    target = os.path.realpath(os.path.join(base, request.GET["p"]))
    if os.path.commonpath([base, target]) != base:
        raise Http404
    return FileResponse(open(target, "rb"))


def contained_sep(request):
    target = os.path.realpath(os.path.join(MEDIA, request.GET["p"]))
    if not target.startswith(MEDIA + os.sep):
        raise Http404
    return FileResponse(open(target, "rb"))


def contained_sep_name(request):
    file_path = os.path.abspath(os.path.join(MEDIA, request.GET["p"]))
    media_root = os.path.abspath(MEDIA) + os.sep
    if not file_path.startswith(media_root):
        raise Http404
    return FileResponse(open(file_path, "rb"))


def contained_resolve(request):
    base = Path(MEDIA).resolve()
    target = (base / request.GET["p"]).resolve()
    if not target.is_relative_to(base):
        raise Http404
    return HttpResponse(target.read_bytes())


def name_only(request):
    name = Path(request.GET["name"]).name
    return HttpResponse((Path(MEDIA) / name).read_text())


def allow_listed(request):
    name = request.GET["name"]
    if name not in os.listdir(MEDIA):
        raise Http404
    return FileResponse(open(os.path.join(MEDIA, name), "rb"))


def get_doc_checked(request):
    return FileResponse(open(storage.resolve_safe(request.GET["doc"]), "rb"))


def get_doc_located(request):
    return FileResponse(open(storage.locate(request.GET["doc"]), "rb"))
