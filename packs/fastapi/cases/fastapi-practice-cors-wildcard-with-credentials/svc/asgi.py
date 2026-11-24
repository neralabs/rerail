from fastapi import FastAPI
from starlette.applications import Starlette
from starlette.middleware import Middleware
from starlette.middleware.cors import CORSMiddleware as Cors

middleware = [
    Middleware(Cors, allow_origins=["*"], allow_credentials=True),  # caught
    Middleware(Cors, allow_origins=["https://example.com"], allow_credentials=True),
    Middleware(Cors, allow_origins=["*"]),
]
app = Starlette(middleware=middleware)
api = FastAPI(middleware=[Middleware(Cors, allow_origin_regex="^.*$", allow_credentials=True)])  # caught

# Wrapping an app in the middleware directly.
application = Cors(app, allow_origins=["*"], allow_credentials=True)  # caught
public = Cors(app, allow_origins=["*"])
