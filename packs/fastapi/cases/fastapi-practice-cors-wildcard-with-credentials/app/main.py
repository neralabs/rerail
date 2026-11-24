import os

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from starlette.middleware.gzip import GZipMiddleware

from app.core import config
from app.core.config import (
    ALL_ORIGINS,
    CREDENTIALS,
    ENV_ORIGINS,
    ENV_ORIGINS_SAFE,
    NO_CREDENTIALS,
    explicit_settings,
    get_settings,
    prod_settings,
    settings,
)
from app.middleware import BrowserCORSMiddleware, CORSConfig

app = FastAPI()

# The literal wildcard with credentials.
app.add_middleware(  # caught
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)
app.add_middleware(CORSMiddleware, allow_origins=("*",), allow_credentials=True)  # caught
app.add_middleware(CORSMiddleware, allow_origins="*", allow_credentials=True)  # caught
app.add_middleware(CORSMiddleware, allow_origins=["https://app.example.com", "*"], allow_credentials=True)  # caught
# A constant of another module, and credentials through a name.
app.add_middleware(CORSMiddleware, allow_origins=ALL_ORIGINS, allow_credentials=CREDENTIALS)  # caught
local_origins = ["*"]
app.add_middleware(CORSMiddleware, allow_origins=local_origins, allow_credentials=True)  # caught
app.add_middleware(CORSMiddleware, allow_origins=config.PUBLIC_ORIGINS + ["*"], allow_credentials=True)  # caught
# A port pattern is no wildcard to Starlette, which compares origins as strings.
app.add_middleware(CORSMiddleware, allow_origins=config.PUBLIC_ORIGINS + ["http://localhost:*"], allow_credentials=True)

# The wildcard as an environment variable's default, through a subclass of CORSMiddleware.
app.add_middleware(BrowserCORSMiddleware, allow_origins=ENV_ORIGINS, allow_credentials=True)  # caught
app.add_middleware(CORSMiddleware, allow_origins=config.ENV_ORIGIN_STRING, allow_credentials=True)  # caught
app.add_middleware(  # caught
    CORSMiddleware,
    allow_origins=os.getenv("ORIGINS", "*").split(","),
    allow_credentials=True,
)

# A settings field whose class default is the wildcard.
app.add_middleware(CORSMiddleware, allow_origins=settings.CORS_ORIGINS, allow_credentials=True)  # caught
app.add_middleware(CORSMiddleware, allow_origins=config.settings.CORS_ORIGINS_FIELD, allow_credentials=True)  # caught
app.add_middleware(CORSMiddleware, allow_origins=get_settings().CORS_ORIGINS, allow_credentials=True)  # caught

# A regular expression that matches every origin.
app.add_middleware(CORSMiddleware, allow_origin_regex=".*", allow_credentials=True)  # caught
app.add_middleware(CORSMiddleware, allow_origin_regex=config.ANY_HTTPS, allow_credentials=True)  # caught

# Explicit origins: left alone.
app.add_middleware(CORSMiddleware, allow_origins=[settings.FRONTEND_HOST], allow_credentials=True)
app.add_middleware(CORSMiddleware, allow_origins=["http://localhost:3000"], allow_credentials=True)
app.add_middleware(CORSMiddleware, allow_origins=settings.BACKEND_CORS_ORIGINS, allow_credentials=True)
app.add_middleware(CORSMiddleware, allow_origins=settings.ALLOWED, allow_credentials=True)
app.add_middleware(CORSMiddleware, allow_origins=ENV_ORIGINS_SAFE, allow_credentials=True)
# A subclass and an instance that set the field: left alone.
app.add_middleware(CORSMiddleware, allow_origins=prod_settings.CORS_ORIGINS, allow_credentials=True)
app.add_middleware(CORSMiddleware, allow_origins=explicit_settings.CORS_ORIGINS, allow_credentials=True)
# A project's own regular expression.
app.add_middleware(CORSMiddleware, allow_origin_regex=r"https://.*\.example\.com", allow_credentials=True)


def load_origins():
    return os.environ["ORIGINS"].split(",")


def setup(app: FastAPI, cfg: dict):
    # Values the rule cannot settle: left alone.
    app.add_middleware(CORSMiddleware, allow_origins=cfg["CORS_origins"], allow_credentials=True)
    app.add_middleware(CORSMiddleware, allow_origins=[str(o) for o in load_origins()], allow_credentials=True)
    app.add_middleware(CORSMiddleware, allow_origins=["*"], allow_credentials=cfg["credentials"])


# The wildcard without credentials: a public API, left alone.
app.add_middleware(CORSMiddleware, allow_origins=["*"])
app.add_middleware(CORSMiddleware, allow_origins=["*"], allow_credentials=False)
app.add_middleware(CORSMiddleware, allow_origins=ALL_ORIGINS, allow_credentials=NO_CREDENTIALS)
app.add_middleware(CORSMiddleware, allow_origin_regex=".*")

# Not CORSMiddleware: left alone.
app.add_middleware(GZipMiddleware, minimum_size=1000)
cors = CORSConfig(allow_origins=["*"], allow_credentials=True)
