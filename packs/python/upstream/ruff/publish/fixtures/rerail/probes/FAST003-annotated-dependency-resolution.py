from typing import Annotated
from fastapi import FastAPI, Depends, Path

app = FastAPI()


def with_id(id): ...
def without(other): ...


class Init:
    def __init__(self, /, id, other): ...


@app.get("/a/{id}")
def a(x: Annotated[str,  # comment
    Path(alias="id")]): ...


@app.get("/b/{id}")
def b(x: Annotated[int, Depends(lambda: 0), Depends(without)]): ...


@app.get("/c/{id}")
def c(x: Annotated[int, Depends(without), Depends(without)]): ...


@app.get("/d/{id}")
def d(x: Annotated[int, Depends(with_id)] = Depends(without)): ...


@app.get("/e/{id}")
def e(x: Annotated[int, Depends(with_id)] = Depends(lambda: 1)): ...


@app.get("/f/{id}")
def f(x=Depends(Init)): ...
