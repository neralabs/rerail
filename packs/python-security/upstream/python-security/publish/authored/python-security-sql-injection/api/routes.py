from fastapi import APIRouter, Depends
from sqlalchemy import text
from sqlalchemy.orm import Session

from api.db import engine, get_session

router = APIRouter()


@router.get("/users")
def find_users(name: str, session: Session = Depends(get_session)):
    rows = session.execute(text(f"SELECT id FROM users WHERE name = '{name}'")).all()  # caught
    rows += session.execute(text("SELECT id FROM users WHERE name = :n").bindparams(n=name)).all()
    rows += session.execute(text("SELECT id FROM users WHERE name = :n"), {"n": name}).all()
    return rows


@router.get("/legacy")
def legacy(email: str, session: Session = Depends(get_session)):
    rows = session.execute("SELECT id FROM users WHERE email = '%s'" % email).all()  # caught
    with engine.connect() as conn:
        rows += conn.exec_driver_sql("SELECT id FROM users WHERE email = '" + email + "'").all()  # caught
        rows += conn.exec_driver_sql("SELECT id FROM users WHERE email = %(e)s", {"e": email}).all()
    return rows


@router.get("/count/{limit}")
def count(limit: int, session: Session = Depends(get_session)):
    return session.execute(text(f"SELECT id FROM users LIMIT {limit}")).all()
