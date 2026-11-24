from sqlalchemy import insert

from app.db import engine
from app.models import users


def test_insert_is_rolled_back() -> None:
    with engine.connect() as conn:
        conn.execute(insert(users).values(name="ghost"))
