from enum import Enum
from typing import Annotated, Literal, Optional

from fastapi import APIRouter, Depends, Path, Query
from pydantic import BaseModel, Field
from sqlalchemy import text
from sqlalchemy.orm import Session

from api.db import get_session

router = APIRouter()


class SortDir(str, Enum):
    asc = "asc"
    desc = "desc"


class Search(BaseModel):
    sort_dir: Literal["asc", "desc"] = "asc"
    order: Optional[SortDir] = None
    limit: int = 10
    column: str = Field("name", pattern="^(name|price)$")
    q: str = ""


@router.get("/a")
def query_pattern(order: str = Query("asc", pattern="^(asc|desc)$"), session: Session = Depends(get_session)):
    return session.execute(text(f"SELECT * FROM t ORDER BY id {order}")).all()


@router.get("/b/{column}")
def path_pattern(column: Annotated[str, Path(pattern="^(name|price)$")], session: Session = Depends(get_session)):
    return session.execute(text(f"SELECT * FROM t ORDER BY {column}")).all()


@router.get("/c")
def loose_pattern(column: str = Query("name", pattern="^[a-z_]+$"), session: Session = Depends(get_session)):
    # Any lower-case identifier passes: the client still picks the column.
    return session.execute(text(f"SELECT * FROM t ORDER BY {column}")).all()  # caught


@router.get("/d")
def enum_param(order: SortDir, limit: int = 10, session: Session = Depends(get_session)):
    return session.execute(text(f"SELECT * FROM t ORDER BY id {order.value} LIMIT {limit}")).all()


@router.post("/e")
def model_fields(body: Search, session: Session = Depends(get_session)):
    rows = session.execute(text(f"SELECT * FROM t ORDER BY {body.column} {body.sort_dir} LIMIT {body.limit}")).all()
    rows += session.execute(text(f"SELECT * FROM t ORDER BY id {body.order}")).all()
    rows += session.execute(text(f"SELECT * FROM t WHERE name = '{body.q}'")).all()  # caught
    return rows
