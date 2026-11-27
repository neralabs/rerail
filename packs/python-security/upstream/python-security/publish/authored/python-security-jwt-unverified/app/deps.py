from typing import Annotated

import jwt
from fastapi import APIRouter, Depends, HTTPException
from fastapi.security import OAuth2PasswordBearer
from sqlmodel import Session

from app.db import get_session
from app.models import User

router = APIRouter()
oauth2 = OAuth2PasswordBearer(tokenUrl="token")
SECRET_KEY = "change-me"
ALGORITHM = "HS256"


def current_account(token: Annotated[str, Depends(oauth2)], session: Session = Depends(get_session)) -> User:
    payload = jwt.decode(token, SECRET_KEY, algorithms=[ALGORITHM], options={"verify_signature": False})  # caught
    user = session.get(User, payload["sub"])
    if not user:
        raise HTTPException(status_code=404)
    return user


def signed_account(token: Annotated[str, Depends(oauth2)], session: Session = Depends(get_session)) -> User:
    payload = jwt.decode(token, SECRET_KEY, algorithms=[ALGORITHM])
    return session.get(User, payload["sub"])


def require_admin(token: Annotated[str, Depends(oauth2)]) -> None:
    claims = jwt.decode(token, options={"verify_signature": False})  # caught
    if claims.get("role") != "admin":
        raise HTTPException(status_code=403)


@router.get("/me", dependencies=[Depends(require_admin)])
def me(user: Annotated[User, Depends(current_account)]) -> User:
    return user


@router.get("/signed")
def signed(user: Annotated[User, Depends(signed_account)]) -> User:
    return user
