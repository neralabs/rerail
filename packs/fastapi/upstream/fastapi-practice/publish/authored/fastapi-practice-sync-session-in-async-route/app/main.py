from fastapi import Depends, FastAPI, Request
from sqlalchemy.orm import Session, scoped_session, sessionmaker

from .db import SessionLocal, engine, get_db
from .routes import router

app = FastAPI()
app.include_router(router)


@app.middleware("http")
async def db_session_middleware(request: Request, call_next):
    request.state.db = SessionLocal()
    request.state.cache = {}
    hit = request.state.cache.get("user")
    response = await call_next(request)
    request.state.db.commit()  # caught
    request.state.db.close()  # caught
    return response, hit


@app.middleware("http")
async def scoped_middleware(request: Request, call_next):
    maker = scoped_session(sessionmaker(bind=engine))
    request.state.scoped = maker()
    try:
        return await call_next(request)
    finally:
        request.state.scoped.remove()  # caught


def mount_late(app: FastAPI):
    app.get("/late")(late_route)


async def late_route(db: Session = Depends(get_db)):
    db.commit()  # caught
