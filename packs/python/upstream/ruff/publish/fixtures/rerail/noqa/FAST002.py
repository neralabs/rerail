from fastapi import FastAPI, Query
app = FastAPI()

@app.get("/")
def f(a: int = Query(1)): ...

from fastapi import FastAPI, Query
app = FastAPI()

@app.get("/")
def f(a: int = Query(1)): ...  # noqa

from fastapi import FastAPI, Query
app = FastAPI()

@app.get("/")
def f(a: int = Query(1)): ...  # noqa: FAST002

from fastapi import FastAPI, Query
app = FastAPI()

@app.get("/")
def f(a: int = Query(1)): ...  # NOQA: FAST002

from fastapi import FastAPI, Query
app = FastAPI()

@app.get("/")
def f(a: int = Query(1)): ...  # noqa: E501
