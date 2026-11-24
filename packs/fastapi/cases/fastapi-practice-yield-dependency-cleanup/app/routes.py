from fastapi import APIRouter, Depends

from .db import SessionLocal
from .deps import AsyncSessionDep, ConnDep, SessionDep, UploadDirDep

router = APIRouter()


# A class-based dependency: FastAPI runs the instance's `__call__`.
class ReadOnlySession:
    def __init__(self, label: str) -> None:
        self.label = label

    def __call__(self):
        db = SessionLocal()
        db.execute("SET TRANSACTION READ ONLY")
        yield db
        db.close()  # caught


@router.get("/items")
def list_items(db: SessionDep, conn: ConnDep):
    return db.execute("select 1").all()


@router.get("/reports")
async def reports(session: AsyncSessionDep, upload_dir: UploadDirDep, ro=Depends(ReadOnlySession("reports"))):
    return {"dir": upload_dir}
