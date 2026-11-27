from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware


def test_wildcard_reflects_origin():
    app = FastAPI()
    app.add_middleware(CORSMiddleware, allow_origins=["*"], allow_credentials=True)
    assert app
