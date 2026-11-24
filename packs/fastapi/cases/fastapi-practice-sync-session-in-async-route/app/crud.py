from contextlib import contextmanager
from functools import cached_property

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Session

from .db import SessionLocal
from .models import User


def get_users(db: Session, skip: int = 0, limit: int = 100):
    return db.query(User).offset(skip).limit(limit).all()


def get_user(db: Session, user_id: int):
    return db.get(User, user_id)


def create_user(db: Session, user):
    db.add(user)
    db.commit()
    db.refresh(user)
    return user


def stage_user(db: Session, user):
    db.add(user)
    db.add_all([user])
    db.expunge(user)
    return user


def first_page(db: Session):
    return get_users(db, 0, 10)


def list_names(session):
    return [u.name for u in session.execute(select(User)).scalars()]


def count_users():
    with SessionLocal() as s:
        return s.scalar(select(User.id))


def later(db: Session):
    def run():
        return db.query(User).all()

    return run


async def aget_users(db: AsyncSession):
    result = await db.execute(select(User))
    return result.scalars().all()


class UserRepo:
    def __init__(self, session: Session):
        self.session = session

    def all(self):
        return self.session.scalars(select(User)).all()

    def name(self):
        return "users"


def open_session() -> Session:
    return SessionLocal()


@contextmanager
def session_scope():
    s = SessionLocal()
    try:
        yield s
    finally:
        s.close()


def build_handler(org: str):
    s = open_session()
    rows = s.query(User).filter(User.org == org).all()
    s.close()
    return rows


def audit():
    with session_scope() as s:
        s.flush()


def make_only():
    return open_session()


class Repos:
    def __init__(self, session: Session):
        self.session = session

    @cached_property
    def users(self) -> UserRepo:
        return UserRepo(self.session)


class BaseRepo[T]:
    session: Session

    def __init__(self, session: Session):
        self.session = session

    def first(self):
        return self.session.scalars(select(User)).first()


class UserRepo2(BaseRepo[User]):
    pass


class Repos2:
    def __init__(self, session: Session):
        self.session = session

    @cached_property
    def users(self) -> UserRepo2:
        return UserRepo2(self.session)
