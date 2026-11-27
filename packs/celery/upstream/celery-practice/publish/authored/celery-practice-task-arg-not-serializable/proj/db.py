from typing import Annotated

from fastapi import Depends
from flask_sqlalchemy import SQLAlchemy
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, scoped_session, sessionmaker

db = SQLAlchemy()
engine = create_engine("sqlite://")
SessionLocal = sessionmaker(bind=engine)
ScopedSession = scoped_session(SessionLocal)


def get_session():
    session = SessionLocal()
    try:
        yield session
    finally:
        session.close()


def new_session():
    return Session(engine)


SessionDep = Annotated[Session, Depends(get_session)]
