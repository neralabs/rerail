from fastapi import FastAPI
from fastapi.testclient import TestClient

app = FastAPI()


@app.get("/{name}")
def by_name(name: str):
    return {}


@app.get("/me")
def me():
    return {}


def test_shadowed():
    assert TestClient(app).get("/me").status_code == 200
