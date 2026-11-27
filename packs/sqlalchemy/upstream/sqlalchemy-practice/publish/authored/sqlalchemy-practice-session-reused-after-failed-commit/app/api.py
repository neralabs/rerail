from fastapi import APIRouter, Depends
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from app.db import SessionLocal
from app.models import Item

router = APIRouter()


def get_session():
    with SessionLocal() as session:
        yield session


@router.post("/items")
def create_item(name: str, session: Session = Depends(get_session)):
    # the dependency closes the session after the response; nothing in the route uses it again
    try:
        session.add(Item(name=name))
        session.commit()
    except IntegrityError:
        return {"ok": False}
    return {"ok": True}


def create_item_helper(name: str, session: Session):
    try:
        session.add(Item(name=name))
        session.commit()
    except IntegrityError:  # caught
        return None
    return name


@router.post("/items/bulk")
def create_items(names: list[str], session: Session = Depends(get_session)):
    created = [create_item_helper(name, session) for name in names]
    session.commit()
    return {"created": created}
