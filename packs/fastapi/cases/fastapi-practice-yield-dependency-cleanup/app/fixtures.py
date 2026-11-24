import pytest

from .db import SessionLocal


# Not a dependency: a pytest fixture with the tutorial's unsafe shape.
@pytest.fixture
def db_session():
    db = SessionLocal()
    yield db
    db.close()
