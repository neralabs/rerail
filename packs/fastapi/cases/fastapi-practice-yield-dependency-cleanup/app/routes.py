from fastapi import APIRouter, Depends

from .db import SessionLocal, scoped_session
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


# The instance bound to a name and the name passed to Depends: FastAPI's docs' own form.
class WriteSession:
    def __call__(self):
        db = SessionLocal()
        yield db
        db.close()  # caught


write_session = WriteSession()


@router.get("/items")
def list_items(db: SessionDep, conn: ConnDep):
    return db.execute("select 1").all()


@router.get("/reports")
async def reports(
    session: AsyncSessionDep,
    upload_dir: UploadDirDep,
    ro=Depends(ReadOnlySession("reports")),
    rw=Depends(write_session),
    sc=Depends(scoped_session),
):
    return {"dir": upload_dir}
