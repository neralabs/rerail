import shutil
from pathlib import Path

from fastapi import APIRouter, HTTPException, UploadFile
from fastapi.responses import FileResponse

from api import seeding, uploads
from api.paths import describe, is_file_in_dir, is_path_in_dir
from api.schemas import Registration

router = APIRouter()
DATA = Path("/srv/data")


@router.get("/media/{file_name}")
async def media(file_name: str):
    return FileResponse(DATA / "media" / file_name)  # caught


@router.get("/export")
async def export(path: str):
    return FileResponse(path=str(DATA / path))  # caught


@router.delete("/media/{file_name}")
async def delete_media(file_name: str):
    (DATA / "media" / file_name).unlink()  # caught


@router.delete("/kb/{kb_name}")
async def delete_kb(kb_name: str):
    shutil.rmtree(DATA / "kb" / kb_name)  # caught


@router.post("/upload")
async def upload(file: UploadFile):
    return await uploads.store(file, DATA / "incoming")


@router.post("/upload-named")
async def upload_named(file: UploadFile):
    return await uploads.store_named(file, DATA / "incoming")


@router.get("/logs/{log_id}")
async def log(log_id: int):
    return FileResponse(DATA / f"{log_id}.log")


@router.get("/tags/{kind}")
async def tag_icon(kind: str):
    if not kind.isalnum():
        raise HTTPException(status_code=400)
    return FileResponse(DATA / "tags" / f"{kind}.png")


@router.post("/files")
async def upload_prefixed(file: UploadFile, folder: str = "tags"):
    if not folder.isalnum():
        raise HTTPException(status_code=400)
    with open(f"{DATA}/{folder}/upload_{file.filename}", "wb") as out:  # caught
        out.write(await file.read())


@router.get("/ui/{rest_of_path:path}")
async def ui(rest_of_path: str):
    base = (DATA / "ui").resolve()
    target = (base / rest_of_path).resolve()
    if target.is_file() and target.is_relative_to(base):
        return FileResponse(str(target))
    return FileResponse(str(base / "index.html"))


@router.get("/results/{name}")
async def result(name: str):
    target = DATA / "results" / name
    if not is_path_in_dir(target, DATA / "results"):
        raise HTTPException(status_code=404)
    return FileResponse(target)


@router.delete("/results/{name}")
async def delete_result(name: str):
    target = (DATA / "results" / name).with_suffix(".json")
    if not is_file_in_dir(target, DATA / "results"):
        raise HTTPException(status_code=404)
    target.unlink()


@router.get("/described/{name}")
async def described(name: str):
    target = DATA / "results" / name
    if not describe(target, DATA / "results"):
        raise HTTPException(status_code=404)
    return FileResponse(target)  # caught


@router.post("/register")
async def register(data: Registration):
    return seeding.seed(data)
