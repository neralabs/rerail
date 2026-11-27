import sqlalchemy as sa
from fastapi import APIRouter, Depends
from sqlalchemy import literal_column, select, text
from sqlalchemy.orm import Session

from api.db import get_session

router = APIRouter()
SORTS = {"name": "users.name", "newest": "users.created DESC"}
ALLOWED = {"name", "email"}


@router.get("/sorted")
def sorted_users(sort: str, session: Session = Depends(get_session)):
    return session.execute(select(literal_column("id")).order_by(text(sort))).all()  # caught


@router.get("/filtered")
def filtered(clause: str, col: str, session: Session = Depends(get_session)):
    stmt = select(literal_column("id")).where(sa.text(clause))  # caught
    stmt = stmt.add_columns(literal_column(col))  # caught
    return session.execute(stmt).all()


@router.get("/mapped")
def mapped(sort: str, session: Session = Depends(get_session)):
    # Mapped through a dict: fixed SQL only.
    return session.execute(select(literal_column("id")).order_by(text(SORTS[sort]))).all()


@router.get("/checked")
def checked(sort: str, session: Session = Depends(get_session)):
    if sort not in ALLOWED:
        raise ValueError(sort)
    return session.execute(select(literal_column("id")).order_by(text(sort))).all()


@router.post("/lab")
def sql_lab(sql: str, session: Session = Depends(get_session)):
    # The client's own statement run as written: an SQL-lab feature, by design.
    return session.execute(text(sql)).all()
