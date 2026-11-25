import os
import uuid
from datetime import datetime
from pathlib import Path

import pandas as pd
from fastapi import APIRouter, Depends, UploadFile
from PIL import Image

router = APIRouter()
BASE = "/srv/uploads"


def current_user() -> dict:
    return {"username": "alice"}


# The client's text after a server name in the same segment: `x/../../y` must pass through a directory
# `<uuid>.x`, which does not exist, so the kernel refuses the path.
@router.post("/avatar")
async def avatar(file: UploadFile):
    ext = file.filename.rsplit(".", 1)[-1].lower()
    with open(os.path.join(BASE, f"{uuid.uuid4().hex}.{ext}"), "wb") as fh:
        fh.write(await file.read())


@router.post("/avatar-split")
async def avatar_split(file: UploadFile):
    ext = file.filename.split(".")[-1]
    with open(f"{BASE}/{uuid.uuid4().hex}.{ext}", "wb") as fh:
        fh.write(await file.read())


@router.post("/avatar-partition")
async def avatar_partition(file: UploadFile):
    ext = file.filename.rpartition(".")[2]
    Path(BASE, "avatar-" + uuid.uuid4().hex + "." + ext).write_bytes(await file.read())


@router.post("/avatar-splitext")
async def avatar_splitext(file: UploadFile):
    ext = os.path.splitext(file.filename)[1]
    with open(os.path.join(BASE, f"{uuid.uuid4().hex}{ext}"), "wb") as fh:
        fh.write(await file.read())


@router.post("/avatar-suffix")
async def avatar_suffix(file: UploadFile):
    with open(os.path.join(BASE, uuid.uuid4().hex + Path(file.filename).suffix), "wb") as fh:
        fh.write(await file.read())


@router.get("/crop")
async def crop(output_format: str = "png"):
    filename = f"cropped_{uuid.uuid4().hex[:8]}_{datetime.now().strftime('%Y%m%d%H%M%S')}.{output_format}"
    with open(os.path.join("./data/temp", filename), "wb") as fh:
        fh.write(b"")


@router.post("/files")
async def upload_user_file(file: UploadFile, user: dict = Depends(current_user)):
    file_name = f"{user['username']}_{datetime.now().strftime('%Y%m%d%H%M%S')}_{file.filename}"
    with open(f"{BASE}/{file_name}", "wb") as fh:
        fh.write(await file.read())


# The same text where the missing directory is no obstacle: a lexical normalisation folds `x/..` first, or the
# missing directories are created.
@router.post("/files-normalised")
async def upload_normalised(file: UploadFile, user: dict = Depends(current_user)):
    file_name = f"{user['username']}_{file.filename}"
    with open(os.path.normpath(os.path.join(BASE, file_name)), "wb") as fh:  # caught
        fh.write(await file.read())


@router.post("/files-resolved")
async def upload_resolved(file: UploadFile):
    target = (Path(BASE) / f"upload_{file.filename}").resolve()
    target.write_bytes(await file.read())  # caught


@router.post("/batch")
async def batch(file: UploadFile):
    folder = os.path.join(BASE, f"batch_{file.filename}")
    os.makedirs(folder, exist_ok=True)  # caught


@router.post("/by-extension")
async def by_extension(file: UploadFile):
    ext = file.filename.rsplit(".", 1)[-1]
    with open(f"{BASE}/{ext}/{uuid.uuid4().hex}", "wb") as fh:  # caught
        fh.write(await file.read())


# An upload's own file object is its content, not a path.
@router.post("/sheet")
async def sheet(file: UploadFile):
    return pd.read_csv(file.file).shape


@router.post("/image")
async def image(file: UploadFile):
    return Image.open(file.file).size


@router.get("/img")
def miss_pil(path: str):
    return Image.open(os.path.join(BASE, path)).size  # caught


@router.get("/csv")
def miss_pandas(path: str):
    return pd.read_csv(os.path.join(BASE, path)).to_dict()  # caught
