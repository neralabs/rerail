from fastapi import Depends, FastAPI
from fastapi.testclient import TestClient

from app.db import SessionLocal

app = FastAPI()


# Test code is left alone, even a real dependency with the unsafe shape.
def override_get_db():
    db = SessionLocal()
    yield db
    db.close()


@app.get("/ping")
def ping(db=Depends(override_get_db)):
    return {"ok": True}


def test_ping():
    assert TestClient(app).get("/ping").status_code == 200
