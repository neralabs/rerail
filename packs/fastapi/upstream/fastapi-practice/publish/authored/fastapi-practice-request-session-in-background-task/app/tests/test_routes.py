from fastapi import BackgroundTasks, Depends, FastAPI
from sqlalchemy.orm import Session

from app.db import get_db
from app.tasks import send_report

app = FastAPI()


@app.post("/t")
def route_under_test(bg: BackgroundTasks, db: Session = Depends(get_db)):
    bg.add_task(send_report, db, 1)
    return {"ok": True}
