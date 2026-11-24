from sqlalchemy.exc import IntegrityError

from app.db import SessionLocal
from app.models import Item


def test_duplicates_are_skipped():
    with SessionLocal() as session:
        for name in ["a", "a"]:
            try:
                session.add(Item(name=name))
                session.commit()
            except IntegrityError:
                continue
