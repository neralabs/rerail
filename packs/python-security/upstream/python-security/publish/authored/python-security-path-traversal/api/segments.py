import os
from pathlib import Path
from urllib.parse import unquote

from fastapi import APIRouter, HTTPException
from fastapi.responses import FileResponse

from api.cache import ContentCache

router = APIRouter()
BASE = "/srv/data"
REPORTS = Path("/srv/reports")
cache = ContentCache()


# A path parameter holds no `/`: beside fixed text in its segment it cannot be `..`.
@router.get("/reports/{name}")
def report_json(name: str):
    return FileResponse(os.path.join(BASE, f"{name}.json"))


@router.get("/reports/v/{name}")
def report_versioned(name: str):
    return FileResponse(REPORTS / f"v_{name}")


@router.get("/reports/csv/{name}")
def report_plus(name: str):
    return FileResponse(os.path.join(BASE, name + ".csv"))


@router.get("/reports/fmt/{name}")
def report_format(name: str):
    return FileResponse(os.path.join(BASE, "report-%s" % name))


@router.get("/reports/raw/{name}")
def report_raw(name: str):
    return FileResponse(os.path.join(BASE, name))  # caught


@router.get("/reports/dot/{name}")
def report_dot(name: str):
    return FileResponse(os.path.join(BASE, f"{name}."))  # caught


@router.get("/reports/any/{name:path}")
def report_any(name: str):
    return FileResponse(os.path.join(BASE, f"{name}.json"))  # caught


@router.get("/reports-by-query")
def report_query(name: str):
    return FileResponse(os.path.join(BASE, f"{name}.json"))  # caught


@router.get("/reports/enc/{name}")
def report_unquoted(name: str):
    return FileResponse(os.path.join(BASE, f"{unquote(name)}.json"))  # caught


@router.get("/reports/swap/{name}")
def report_swapped(name: str):
    return FileResponse(os.path.join(BASE, name.replace("~", "/") + ".json"))  # caught


# Its only climb is `..`: `startswith(base)` on the resolved path stops it (we-mp-rss download_cropped_image).
@router.get("/cropped/{filename}")
def cropped(filename: str):
    temp_dir = os.path.abspath("./data/temp")
    safe_path = os.path.abspath(os.path.join(temp_dir, filename))
    if not safe_path.startswith(temp_dir):
        raise HTTPException(status_code=403)
    if not os.path.exists(safe_path):
        raise HTTPException(status_code=404)
    os.remove(safe_path)
    return FileResponse(safe_path)


@router.get("/cropped")
def cropped_query(filename: str):
    temp_dir = os.path.abspath("./data/temp")
    safe_path = os.path.abspath(os.path.join(temp_dir, filename))
    if not safe_path.startswith(temp_dir):
        raise HTTPException(status_code=403)
    if not os.path.exists(safe_path):
        raise HTTPException(status_code=404)
    os.remove(safe_path)  # caught
    return FileResponse(safe_path)  # caught


@router.get("/notes/{name}")
def note(name: str):
    if ".." in name:
        raise HTTPException(status_code=400)
    return FileResponse(os.path.join(BASE, name))


@router.get("/content/{content_id}")
def content(content_id: str):
    return cache.get(content_id)


@router.get("/content/{content_id}/raw")
def content_raw(content_id: str):
    return cache.raw(content_id)
