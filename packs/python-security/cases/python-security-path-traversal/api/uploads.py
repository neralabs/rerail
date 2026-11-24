from pathlib import Path

import aiofiles
from fastapi import UploadFile


async def store(upload: UploadFile, dest: Path):
    target = dest / upload.filename
    async with aiofiles.open(target, "wb") as out:  # caught
        await out.write(await upload.read())
    return {"path": str(target)}


async def store_named(upload: UploadFile, dest: Path):
    (dest / Path(upload.filename).name).write_bytes(await upload.read())
    return {"ok": True}
