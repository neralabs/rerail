from fastapi import APIRouter, Depends
from fastapi.responses import JSONResponse
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from app import crud
from app.deps import CurrentUser, SessionDep, get_current_user
from app.models import ApiToken, Hero, Member, MfaDevice, Service, User
from app.schemas import ServiceWithKey, UserPublic

router = APIRouter()


@router.get("/raw/me")
def raw_me(current_user: CurrentUser):
    return current_user  # caught


@router.get("/raw/me2")
def raw_me_depends(user: User = Depends(get_current_user)):
    return user  # caught


@router.get("/raw/users/{user_id}")
def raw_user(user_id: int, session: SessionDep):
    user = session.get(User, user_id)
    if user is None:
        raise ValueError(user_id)
    return user  # caught


@router.get("/raw/async/{user_id}")
async def raw_user_async(user_id: int, session: AsyncSession):
    user = await session.get(User, user_id)
    return user  # caught


@router.get("/raw/by-email")
def raw_by_email(email: str, session: SessionDep):
    return session.query(User).filter(User.email == email).first()  # caught


@router.get("/raw/select")
async def raw_select(session: AsyncSession):
    result = await session.execute(select(User).limit(10))
    return result.scalars().all()  # caught


@router.get("/raw/exec")
def raw_exec(session: SessionDep):
    return session.exec(select(Service)).first()  # caught


@router.get("/raw/crud")
def raw_crud(email: str, session: SessionDep):
    found = crud.get_user_by_email(session, email)
    return found  # caught


@router.get("/raw/tokens/{token_id}", response_model=None)
def raw_token(token_id: int, session: SessionDep) -> ApiToken:
    return session.get(ApiToken, token_id)  # caught


@router.post("/raw/mfa")
def raw_mfa(session: SessionDep):
    device = MfaDevice(totp_secret="abc")
    session.add(device)
    return device  # caught


@router.get("/raw/list")
def raw_list(session: SessionDep):
    return session.scalars(select(User)).all()  # caught


# Converted before it is returned: the last assignment is the public model.
@router.get("/safe/me")
def safe_me(current_user: CurrentUser):
    user = current_user
    user = UserPublic(id=user.id, email=user.email)
    return user


# A Response built by hand is not run through the response model.
@router.get("/safe/json")
def safe_json(current_user: CurrentUser):
    return JSONResponse({"id": current_user.id})


# A model without a secret field, and a single column of a model that has one.
@router.get("/safe/hero/{hero_id}")
def safe_hero(hero_id: int, session: SessionDep):
    return session.get(Hero, hero_id)


@router.get("/safe/emails")
def safe_emails(session: SessionDep):
    return session.scalars(select(User.email)).all()


@router.get("/safe/count")
def safe_count(session: SessionDep):
    return crud.count_users(session)


# Rows of two entities are tuples, not one model.
@router.get("/safe/pairs")
def safe_pairs(session: SessionDep):
    return session.query(User, Hero).first()


# The request body is echoed back to the client that sent it, nobody else.
@router.post("/safe/echo")
def safe_echo(body: Member):
    return body


# A schema built to show a new key once: key names count only on ORM models.
@router.post("/safe/new-key")
def safe_new_key():
    return ServiceWithKey(name="x", api_key="y")


# Not a route.
def helper(session: SessionDep):
    return session.get(User, 1)
