import functools
from typing import Annotated

import requests
from fastapi import APIRouter, BackgroundTasks, Depends
from fastapi.responses import JSONResponse
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Session
from starlette.background import BackgroundTask

from .db import SessionLocal, get_async_db, get_db
from .deps import SessionDep
from .jobs import queue
from .tasks import ReportService, refresh_stats, send_and_close, send_email, send_report, send_report_by_id

router = APIRouter()


# ---- reported ------------------------------------------------------------------------------------------------------


@router.post("/positional/{user_id}")
def positional(user_id: int, bg: BackgroundTasks, db: Session = Depends(get_db)):
    bg.add_task(send_report, db, user_id)  # caught
    return {"ok": True}


@router.post("/keyword/{user_id}")
def keyword(user_id: int, db: SessionDep, background_tasks: BackgroundTasks):
    background_tasks.add_task(
        send_report,
        user_id=user_id,
        db=db,  # caught
    )
    return {"ok": True}


@router.post("/async/{user_id}")
async def async_session(
    user_id: int,
    bg: BackgroundTasks,
    db: Annotated[AsyncSession, Depends(get_async_db)],
):
    bg.add_task(refresh_stats, db, user_id)  # caught
    return {"ok": True}


@router.post("/untyped/{user_id}")
def untyped(user_id: int, bg: BackgroundTasks, db=Depends(get_db)):
    session = db
    bg.add_task(send_report, session, user_id)  # caught
    return {"ok": True}


@router.post("/response-task/{user_id}")
def response_task(user_id: int, db: SessionDep):
    return JSONResponse({"ok": True}, background=BackgroundTask(send_report, db, user_id))  # caught


@router.post("/lambda/{user_id}")
def in_lambda(user_id: int, db: SessionDep, bg: BackgroundTasks):
    bg.add_task(lambda: send_report(db, user_id))  # caught
    return {"ok": True}


@router.post("/closure/{user_id}")
def in_closure(user_id: int, db: SessionDep, bg: BackgroundTasks):
    def job() -> None:
        send_report(db, user_id)

    bg.add_task(job)  # caught
    return {"ok": True}


@router.post("/partial/{user_id}")
def in_partial(user_id: int, db: SessionDep, bg: BackgroundTasks):
    bg.add_task(functools.partial(send_report, db), user_id)  # caught
    return {"ok": True}


@router.post("/bound/{user_id}")
def bound_method(user_id: int, db: SessionDep, bg: BackgroundTasks):
    bg.add_task(ReportService(db).run, user_id)  # caught
    service = ReportService(db)
    bg.add_task(service.run, user_id)  # caught
    return {"ok": True}


@router.post("/with/{user_id}")
def with_session(user_id: int, bg: BackgroundTasks):
    with SessionLocal() as s:
        bg.add_task(send_report, s, user_id)  # caught
    return {"ok": True}


def audit(bg: BackgroundTasks, db: SessionDep) -> None:
    bg.add_task(send_report, db, 0)  # caught


@router.get("/audited", dependencies=[Depends(audit)])
def audited():
    return {"ok": True}


# ---- left alone ----------------------------------------------------------------------------------------------------


@router.post("/ids/{user_id}")
def ids_only(user_id: int, db: SessionDep, bg: BackgroundTasks, email: str = ""):
    user = db.get(dict, user_id)
    bg.add_task(send_report_by_id, user_id=user.id)
    bg.add_task(send_report_by_id, user_id)
    bg.add_task(send_email, email)
    return {"ok": True}


@router.post("/own-session/{user_id}")
def own_session(user_id: int, bg: BackgroundTasks):
    def job() -> None:
        with SessionLocal() as db:
            send_report(db, user_id)

    bg.add_task(job)
    bg.add_task(lambda: send_report_by_id(user_id))
    return {"ok": True}


@router.post("/handoff/{user_id}")
def handoff(user_id: int, bg: BackgroundTasks):
    # The route opens this session and the task closes it: no dependency owns it.
    s = SessionLocal()
    bg.add_task(send_and_close, s, user_id)
    return {"ok": True}


@router.post("/queue/{user_id}")
def other_queue(user_id: int, db: SessionDep):
    queue.add_task(send_report, db, user_id)
    return {"ok": True}


@router.post("/http/{user_id}")
def http_session(user_id: int, bg: BackgroundTasks, http: requests.Session):
    bg.add_task(http.post, "https://example.com/hook", json={"id": user_id})
    return {"ok": True}


@router.post("/service-by-id/{user_id}")
def service_without_session(user_id: int, db: SessionDep, bg: BackgroundTasks):
    service = ReportService(SessionLocal())
    bg.add_task(service.run, user_id)
    bg.add_task(send_email, db.info["email"])
    return {"ok": True}


def schedule(bg: BackgroundTasks, db: Session) -> None:
    # A plain helper, not a route or dependency: whose session it holds is up to its caller.
    bg.add_task(send_report, db, 0)
