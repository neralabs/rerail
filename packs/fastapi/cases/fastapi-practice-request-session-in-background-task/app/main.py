from fastapi import BackgroundTasks, FastAPI, Request

from .db import SessionLocal
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
