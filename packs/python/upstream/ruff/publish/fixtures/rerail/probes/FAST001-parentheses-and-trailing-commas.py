from typing import List, Dict, Tuple
from fastapi import FastAPI

app = FastAPI()


class Item: ...


@app.get("/a", response_model=List[(Item)])
def a() -> List[Item]: ...


@app.get("/b", response_model=Dict[str, Item,])
def b() -> Dict[str, Item]: ...


@app.get("/c", response_model=(Item))
def c() -> Item: ...


@app.get("/d", response_model=Item)
def d() -> (Item): ...


@app.get("/e", response_model=List[Item,])
def e() -> List[Item]: ...


@app.get("/f", response_model=Tuple[()])
def f() -> Tuple[()]: ...


@app.get("/g", response_model=List[List[Item]])
def g() -> List[List[(Item)]]: ...


@app.get("/h", response_model=Dict[str, Item])
def h() -> Dict[(str), Item]: ...
