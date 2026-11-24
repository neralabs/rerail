from typing import Annotated

from fastapi import Depends, HTTPException, Security
from fastapi.security import APIKeyHeader, OAuth2PasswordBearer

from app.db import SessionLocal

oauth2_scheme = OAuth2PasswordBearer(tokenUrl="/login/access-token")
api_key_header = APIKeyHeader(name="X-API-Key")


def get_db():
    with SessionLocal() as db:
        yield db


SessionDep = Annotated[object, Depends(get_db)]


def get_current_user(token: str = Depends(oauth2_scheme)):
    if not token:
        raise HTTPException(401)
    return {"id": 1}


CurrentUser = Annotated[dict, Depends(get_current_user)]


def require_admin(user: CurrentUser):
    return user


def load_owner(token: Annotated[str, Depends(oauth2_scheme)]):
    # Its name says nothing; the OAuth2 scheme it reads does.
    return token


def check_key(key: str = Security(api_key_header)):
    return key


def get_pagination(skip: int = 0, limit: int = 100):
    return skip, limit


class _Authenticator:
    def __init__(self, scopes: set[str]):
        self.scopes = scopes

    async def __call__(self, token: str = ""):
        return {"scopes": self.scopes}


def Authenticator(scopes: set[str]) -> _Authenticator:
    return _Authenticator(scopes)


# A dependency instance whose variable says nothing; the factory that builds it does.
_ProjectsWrite = Authenticator({"projects:write"})
ProjectsWrite = Annotated[dict, Depends(_ProjectsWrite)]
