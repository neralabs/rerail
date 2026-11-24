from typing import Annotated

from fastapi import APIRouter, Cookie, Depends, Header, Request
from fastapi.security import OAuth2PasswordRequestForm

from app.deps import CurrentUser, get_current_user

router = APIRouter()


@router.get("/me")
def read_me(user: CurrentUser):
    return user


@router.patch("/me")
def update_me(user: CurrentUser):
    return user


@router.delete("/me")
def delete_me(user: CurrentUser):
    return {}


@router.get("/me/settings")
def read_settings(user: CurrentUser):
    return {}


# Public by design: the name or path says so.
@router.post("/login/access-token")
def login_access_token(form: Annotated[OAuth2PasswordRequestForm, Depends()]):
    return {}


@router.post("/signup")
def register_user(body: dict):
    return {}


@router.post("/password-recovery/{email}")
def recover_password(email: str):
    return {}


@router.post("/reset-password/")
def reset_password(body: dict):
    return {}


@router.post("/stripe/webhook")
async def stripe_events(request: Request):
    return {}


@router.get("/.well-known/jwks.json")
def jwks():
    return {}


# Reads its credentials by hand.
@router.post("/sync")
async def sync_from_agent(request: Request):
    token = request.headers.get("authorization")
    return {"token": token}


@router.post("/ingest")
def ingest(payload: dict, x_signature: str = Header(...)):
    return {}


@router.post("/prefs")
def save_prefs(payload: dict, session_id: Annotated[str | None, Cookie()] = None):
    return {}


@router.post("/upload")
async def upload(request: Request):
    user = await get_current_user(request)
    return {"user": user}


# A machine endpoint guarded by an IP allowlist (notify-server's shape).
@router.post("/services/{service_id}/notifications")
def notify_service(service_id: int, x_forwarded_for: list[str] | None = Header(None)):
    if not check_ips(x_forwarded_for):
        raise ValueError("IP address not allowed")
    return {}


def check_ips(ips):
    return bool(ips)

