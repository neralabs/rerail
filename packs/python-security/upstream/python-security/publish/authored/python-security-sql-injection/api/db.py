from sqlalchemy import create_engine
from sqlalchemy.orm import Session

engine = create_engine("postgresql://localhost/app")


def get_session():
    with Session(engine) as session:
        yield session
