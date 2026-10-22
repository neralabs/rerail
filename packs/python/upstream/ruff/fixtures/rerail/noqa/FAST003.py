from fastapi import FastAPI
app = FastAPI()

@app.get("/things/{thing_id}")
def f(): ...

from fastapi import FastAPI
app = FastAPI()

@app.get("/things/{thing_id}")  # noqa
def f(): ...

from fastapi import FastAPI
app = FastAPI()

@app.get("/things/{thing_id}")  # noqa: FAST003
def f(): ...

from fastapi import FastAPI
app = FastAPI()

@app.get("/things/{thing_id}")  # NOQA: FAST003
def f(): ...

from fastapi import FastAPI
app = FastAPI()

@app.get("/things/{thing_id}")  # noqa: E501
def f(): ...
