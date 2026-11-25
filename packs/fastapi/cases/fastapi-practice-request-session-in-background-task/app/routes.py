import functools
from typing import Annotated

import requests
from fastapi import APIRouter, BackgroundTasks, Depends, Request
from fastapi.responses import JSONResponse
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Session
from starlette.background import BackgroundTask

from .db import SessionLocal, get_async_db, get_db, get_tx
from .deps import FnSessionDep, SessionDep, StateSessionDep
from .jobs import queue
from .tasks import ReportService, refresh_stats, send_and_close, send_email, send_report, send_report_by_id

router = APIRouter()


# ---- reported ------------------------------------------------------------------------------------------------------


@router.post("/positional/{user_id}")
def positional(user_id: int, bg: BackgroundTasks, db: Session = Depends(get_db, scope="function")):
    bg.add_task(send_report, db, user_id)  # caught
    return {"ok": True}


@router.post("/keyword/{user_id}")
def keyword(user_id: int, db: StateSessionDep, background_tasks: BackgroundTasks):
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
    db: Annotated[AsyncSession, Depends(get_async_db, scope="function")],
):
    bg.add_task(refresh_stats, db, user_id)  # caught
    return {"ok": True}


@router.post("/untyped/{user_id}")
def untyped(user_id: int, bg: BackgroundTasks, db=Depends(get_db, scope="function")):
    session = db
    bg.add_task(send_report, session, user_id)  # caught
    return {"ok": True}


@router.post("/response-task/{user_id}")
def response_task(user_id: int, db: FnSessionDep):
    return JSONResponse({"ok": True}, background=BackgroundTask(send_report, db, user_id))  # caught


@router.post("/lambda/{user_id}")
def in_lambda(user_id: int, db: StateSessionDep, bg: BackgroundTasks):
    bg.add_task(lambda: send_report(db, user_id))  # caught
    return {"ok": True}


@router.post("/closure/{user_id}")
def in_closure(user_id: int, db: FnSessionDep, bg: BackgroundTasks):
    def job() -> None:
        send_report(db, user_id)

    bg.add_task(job)  # caught
    return {"ok": True}


@router.post("/partial/{user_id}")
def in_partial(user_id: int, db: StateSessionDep, bg: BackgroundTasks):
    bg.add_task(functools.partial(send_report, db), user_id)  # caught
    return {"ok": True}


@router.post("/bound/{user_id}")
def bound_method(user_id: int, db: FnSessionDep, bg: BackgroundTasks):
    bg.add_task(ReportService(db).run, user_id)  # caught
    service = ReportService(db)
    bg.add_task(service.run, user_id)  # caught
    return {"ok": True}


@router.post("/with/{user_id}")
def with_session(user_id: int, bg: BackgroundTasks):
    with SessionLocal() as s:
        bg.add_task(send_report, s, user_id)  # caught
    return {"ok": True}


def audit(bg: BackgroundTasks, db: StateSessionDep) -> None:
    bg.add_task(send_report, db, 0)  # caught


@router.get("/audited", dependencies=[Depends(audit)])
def audited():
    return {"ok": True}


# ---- left alone ----------------------------------------------------------------------------------------------------


# A default-scope yield dependency's session: on FastAPI 0.118+ it stays open until the background tasks finish.
@router.post("/default-scope/{user_id}")
def default_scope(user_id: int, db: SessionDep, bg: BackgroundTasks):
    bg.add_task(send_report, db, user_id)
    bg.add_task(lambda: send_report(db, user_id))
    return {"ok": True}


@router.post("/default-depends/{user_id}")
async def default_depends(
    user_id: int,
    bg: BackgroundTasks,
    db: Session = Depends(get_db),
    adb: Annotated[AsyncSession, Depends(get_async_db)] = None,
):
    bg.add_task(send_report, db, user_id)
    bg.add_task(refresh_stats, adb, user_id)
    return {"ok": True}


# `.state.tx` is set by a yield dependency, not a middleware: open while the task runs.
@router.post("/dep-state/{user_id}")
def dep_state(user_id: int, request: Request, bg: BackgroundTasks, tx: Session = Depends(get_tx)):
    bg.add_task(send_report, request.state.tx, user_id)
    return {"ok": True}


@router.post("/ids/{user_id}")
def ids_only(user_id: int, db: FnSessionDep, bg: BackgroundTasks, email: str = ""):
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
def other_queue(user_id: int, db: FnSessionDep):
    queue.add_task(send_report, db, user_id)
    return {"ok": True}


@router.post("/http/{user_id}")
def http_session(user_id: int, bg: BackgroundTasks, http: requests.Session):
    bg.add_task(http.post, "https://example.com/hook", json={"id": user_id})
    return {"ok": True}


@router.post("/service-by-id/{user_id}")
def service_without_session(user_id: int, db: FnSessionDep, bg: BackgroundTasks):
    service = ReportService(SessionLocal())
    bg.add_task(service.run, user_id)
    bg.add_task(send_email, db.info["email"])
    return {"ok": True}


def schedule(bg: BackgroundTasks, db: Session) -> None:
    # A plain helper, not a route or dependency: whose session it holds is up to its caller.
    bg.add_task(send_report, db, 0)


# Gap: the helper above, called from a route with a function-scoped session.
@router.post("/via-helper/{user_id}")
def via_helper(user_id: int, db: FnSessionDep, bg: BackgroundTasks):
    schedule(bg, db)
    return {"ok": True}


def get_service(db: FnSessionDep) -> ReportService:
    return ReportService(db)


# Gap: an object a dependency built from the session.
@router.post("/via-service/{user_id}")
def via_service(user_id: int, bg: BackgroundTasks, svc: ReportService = Depends(get_service)):
    bg.add_task(svc.run, user_id)
    return {"ok": True}


# Gap: an ORM object loaded in a `with` block, detached by the time the task reads it.
@router.post("/detached/{user_id}")
def detached(user_id: int, bg: BackgroundTasks):
    with SessionLocal() as s:
        user = s.get(dict, user_id)
    bg.add_task(send_email, user)
    return {"ok": True}
