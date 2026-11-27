from fastapi import FastAPI


class CORSMiddleware:
    """A project's own middleware that happens to share the name: not Starlette's."""

    def __init__(self, app, allow_origins=(), allow_credentials=False):
        self.app = app


app = FastAPI()
app.add_middleware(CORSMiddleware, allow_origins=["*"], allow_credentials=True)
