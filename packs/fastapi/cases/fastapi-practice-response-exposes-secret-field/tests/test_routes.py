from fastapi import APIRouter, FastAPI
from fastapi.testclient import TestClient

from app.models import User
from app.schemas import UserInDB

router = APIRouter()


@router.get("/fixture-user", response_model=UserInDB)
def fixture_user():
    return User(email="a@example.com", hashed_password="x")


def test_fixture_user():
    app = FastAPI()
    app.include_router(router)
    assert TestClient(app).get("/fixture-user").status_code == 200
