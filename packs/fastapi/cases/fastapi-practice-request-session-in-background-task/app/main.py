from fastapi import BackgroundTasks, FastAPI, Request
from starlette.middleware.base import BaseHTTPMiddleware

from .db import SessionLocal
from .deps import StateSessionDep as Db
from .tasks import send_report

app = FastAPI()


@app.middleware("http")
async def db_session_middleware(request: Request, call_next):
    request.state.db = SessionLocal()
    try:
        return await call_next(request)
    finally:
        request.state.db.close()


@app.post("/state/{user_id}")
def state_report(user_id: int, request: Request, bg: BackgroundTasks):
    bg.add_task(send_report, request.state.db, user_id)  # caught
    bg.add_task(send_report, request.state.user, user_id)
    return {"ok": True}


# A BaseHTTPMiddleware subclass that opens the session in a `with` block around call_next.
class TxMiddleware(BaseHTTPMiddleware):
    async def dispatch(self, request, call_next):
        with SessionLocal() as s:
            request.state.session = s
            return await call_next(request)


app.add_middleware(TxMiddleware)


@app.post("/tx/{user_id}")
def tx_report(user_id: int, request: Request, bg: BackgroundTasks, db: Db):
    bg.add_task(send_report, request.state.session, user_id)  # caught
    bg.add_task(send_report, db, user_id)  # caught
    return {"ok": True}
