import asyncio

import anyio
import requests
from fastapi import APIRouter, Depends
from fastapi.concurrency import run_in_threadpool
from sqlalchemy import select
from sqlalchemy.orm import Session

from . import crud
from .crud import UserRepo, audit, build_handler, count_users, create_user, first_page, get_users, list_names, stage_user
from .db import SessionLocal, get_db
from .deps import AsyncSessionDep, HttpDep, SessionDep
from .models import Trade, User

router = APIRouter()


@router.get("/users")
async def users_direct(db=Depends(get_db)):
    users = db.query(User).filter(User.id > 0).all()  # caught
    return users


@router.get("/users/{user_id}")
async def user_alias(user_id: int, session: SessionDep):
    user = session.get(User, user_id)  # caught
    rows = session.exec(select(User)).all()  # caught
    return user, rows


@router.post("/users")
async def user_create(user: dict, db: Session = Depends(get_db)):
    staged = stage_user(db, user)
    db.add(staged)
    return create_user(db, staged)  # caught


@router.get("/users-helpers")
async def users_helpers(db: Session = Depends(get_db)):
    a = get_users(db)  # caught
    b = crud.get_user(db, 1)  # caught
    c = first_page(db)  # caught
    d = list_names(db)  # caught
    e = count_users()  # caught
    return a, b, c, d, e


@router.get("/users-repo")
async def users_repo(db: Session = Depends(get_db)):
    repo = UserRepo(db)
    label = repo.name()
    return label, repo.all()  # caught


@router.get("/own-session")
async def own_session():
    with SessionLocal() as s:
        return s.execute(select(User)).all()  # caught


async def current_user(db: Session = Depends(get_db)):
    return crud.get_user(db, 1)  # caught


async def rpc():
    Trade.rollback()  # caught
    n = Trade.open_count()  # caught
    yield Trade.label()


@router.get("/me", dependencies=[Depends(rpc)])
async def me(user=Depends(current_user)):
    return user


# Near misses: the threadpool, AsyncSession, offloaded or deferred work, no IO, other libraries.


@router.get("/sync")
def users_sync(db: Session = Depends(get_db)):
    db.query(User).all()
    return get_users(db)


@router.get("/offloaded")
async def users_offloaded(db: Session = Depends(get_db)):
    a = await run_in_threadpool(get_users, db)
    b = await asyncio.to_thread(crud.get_user, db, 1)
    c = await anyio.to_thread.run_sync(lambda: first_page(db))
    loop = asyncio.get_running_loop()
    d = await loop.run_in_executor(None, list_names, db)
    run = crud.later(db)
    return a, b, c, d, run


@router.get("/async-session")
async def users_async(session: AsyncSessionDep):
    result = await session.execute(select(User))
    users = await crud.aget_users(session)
    return result.scalars().all(), users


@router.get("/nested")
async def users_nested(db: Session = Depends(get_db)):
    def work():
        return db.query(User).all()

    return await run_in_threadpool(work)


@router.get("/http")
async def http_client(http: HttpDep):
    s = requests.Session()
    s.close()
    return http.get("https://example.com").json()


@router.get("/unknown")
async def unknown(db: Session = Depends(get_db), fn=None):
    return fn(db)


async def background_job():
    with SessionLocal() as s:
        s.commit()


@router.post("/slack/{org}")
async def slack_events(org: str):
    rows = build_handler(org)  # caught
    audit()  # caught
    session = crud.make_only()
    return rows, session


@router.get("/repos")
async def users_repos(db: Session = Depends(get_db)):
    repos = crud.Repos(db)
    label = repos.users.name()
    return label, repos.users.all()  # caught


@router.get("/repos2")
async def users_repos2(db: Session = Depends(get_db)):
    repos = crud.Repos2(db)
    return repos.users.first()  # caught
