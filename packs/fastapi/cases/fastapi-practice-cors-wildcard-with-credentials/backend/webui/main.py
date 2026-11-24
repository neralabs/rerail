from fastapi import FastAPI

from webui.config import CORS_ALLOW_ORIGIN
from webui.utils.asgi import SandboxCORSMiddleware

app = FastAPI()
app.add_middleware(  # caught
    SandboxCORSMiddleware,
    allow_origins=CORS_ALLOW_ORIGIN,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)
app.add_middleware(SandboxCORSMiddleware, allow_origins=["https://app.example.com"], allow_credentials=True)
