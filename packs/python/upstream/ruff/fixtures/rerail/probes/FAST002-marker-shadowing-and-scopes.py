from fastapi import FastAPI, Query, Depends

app = FastAPI()


def outer(Query):
    @app.get("/a")
    def a(q: int = Query(None), d: int = Depends(dep)): ...


class K:
    Depends = staticmethod(lambda f: f)

    @app.get("/b")
    def b(self, d: int = Depends(dep), q: int = Query()): ...


@app.get("/c")
def c(q: int = Query(None)): ...


Query = None


def later():
    @app.get("/d")
    def d(q: int = Query(None), e: int = Depends(dep)): ...
