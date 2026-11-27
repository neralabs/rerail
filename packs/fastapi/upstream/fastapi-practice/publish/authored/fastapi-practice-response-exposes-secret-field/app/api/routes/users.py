from typing import Any, List, Optional

from fastapi import APIRouter

from app import schemas
from app.deps import CurrentUser, SessionDep
from app.models import (
    Device,
    Flagged,
    Hero,
    Hook,
    Integration,
    LegacyLogin,
    Login,
    Mailer,
    Operator,
    Member,
    OAuthAccount,
    Provider,
    Toggles,
    User,
)
from app.schemas import (
    AliasMasked,
    AnnotatedMasked,
    Exported,
    HookCreated,
    LookAlike,
    MaskedChild,
    MaskedOut,
    Page,
    ServiceOut,
    ServiceWithKey,
    SmtpSettings,
    Token,
    UserInDB,
    UserList,
    UserPublic,
    WifiOut,
)

router = APIRouter(prefix="/users")


@router.get("/me", response_model=UserInDB)  # caught
def read_me(current_user: CurrentUser) -> Any:
    return current_user


@router.get("/", response_model=List[UserInDB])  # caught
def list_users(session: SessionDep):
    return session.query(User).all()


@router.get("/page", response_model=Page[schemas.UserInDB])  # caught
def page_users(session: SessionDep):
    return {"items": session.query(User).all(), "total": 0}


@router.get("/all", response_model=UserList)  # caught
def all_users(session: SessionDep):
    return session.query(User).all()


@router.get("/{user_id}", response_model=Optional[Member])  # caught
def read_user(user_id: int, session: SessionDep):
    return session.get(Member, user_id)


@router.get("/oauth/{account_id}", response_model=OAuthAccount)  # caught
def read_oauth_account(account_id: int, session: SessionDep):
    return session.get(OAuthAccount, account_id)


@router.get("/integrations")
def list_integrations(session: SessionDep) -> list[Integration]:  # caught
    return session.exec(Integration).all()


@router.get("/member/{email}")
async def read_member(email: str, session: SessionDep) -> Member | None:  # caught
    return session.query(Member).filter(Member.email == email).first()


@router.post("/devices", response_model=Device, status_code=201)  # caught
def create_device(session: SessionDep):
    return Device(name="x", private_key="y")


# A response schema written to show a new key once is a design choice: key names count only on ORM models.
@router.post("/keyed", response_model=ServiceWithKey)
def create_keyed():
    return ServiceWithKey(name="x", api_key="y")


def admin_view(session: SessionDep):
    return session.query(User).all()


router.add_api_route("/admin", admin_view, response_model=list[UserInDB])  # caught


# The response model hides the field: the safe idiom.
@router.get("/public/me", response_model=UserPublic)
def read_me_public(current_user: CurrentUser) -> Any:
    return current_user


# The return annotation is the response model only when response_model is absent.
@router.get("/public/{user_id}", response_model=UserPublic)
def read_public(user_id: int, session: SessionDep) -> User:
    return session.get(User, user_id)


@router.get("/public", response_model=list[UserPublic])
def list_public(session: SessionDep) -> list[User]:
    return session.query(User).all()


# A login route's token response: access_token is the point of the route.
@router.post("/login", response_model=Token)
def login() -> Token:
    return Token(access_token="a", refresh_token="b")


# A secret field typed SecretStr is masked; a field with Field(exclude=True) is never sent.
@router.get("/smtp", response_model=SmtpSettings)
def read_smtp() -> SmtpSettings:
    return SmtpSettings(host="mail")


@router.get("/service-out", response_model=ServiceOut)
def read_service_out() -> ServiceOut:
    return ServiceOut(name="x", api_key="y")


# response_model_exclude names the field.
@router.get("/excluded/{user_id}", response_model=UserInDB, response_model_exclude={"hashed_password"})
def read_excluded(user_id: int, session: SessionDep):
    return session.get(User, user_id)


@router.get("/included/{user_id}", response_model=UserInDB, response_model_include={"id", "email"})
def read_included(user_id: int, session: SessionDep):
    return session.get(User, user_id)


# `secret_name` and `password_changed_at` are not secret names: whole names only.
@router.get("/heroes/{hero_id}", response_model=Hero)
def read_hero(hero_id: int, session: SessionDep):
    return session.get(Hero, hero_id)


@router.get("/flags", response_model=schemas.UserWithFlags)
def read_flags(current_user: CurrentUser):
    return current_user


# The request body may carry a password: only the response matters.
@router.post("/", response_model=UserPublic)
def create_user(user_in: schemas.UserCreate, session: SessionDep) -> UserPublic:
    return UserPublic(id=1, email=user_in.email)


@router.get("/logins/{login_id}", response_model=Login)  # caught
def read_login(login_id: int, session: SessionDep):
    return session.get(Login, login_id)


@router.get("/legacy/{login_id}", response_model=LegacyLogin)  # caught
def read_legacy_login(login_id: int, session: SessionDep):
    return session.get(LegacyLogin, login_id)


@router.get("/hooks/{hook_id}", response_model=Hook)  # caught
def read_hook(hook_id: int, session: SessionDep):
    return session.get(Hook, hook_id)


@router.get("/providers/{provider_id}")
def read_provider(provider_id: int, session: SessionDep):
    return session.get(Provider, provider_id)  # caught


@router.get("/wifi", response_model=WifiOut)  # caught
def read_wifi() -> WifiOut:
    return WifiOut(ssid="home", password="hunter2")


# Near misses: flags, serializers that decide what is sent, a suffix key on a schema.
@router.get("/flagged/{flag_id}", response_model=Flagged)
def read_flagged(flag_id: int, session: SessionDep):
    return session.get(Flagged, flag_id)


@router.get("/masked", response_model=MaskedOut)
def read_masked() -> MaskedOut:
    return MaskedOut(email="a@example.com", password="x")


@router.get("/masked-child", response_model=MaskedChild)
def read_masked_child() -> MaskedChild:
    return MaskedChild(email="a@example.com", password="x", name="n")


@router.get("/exported", response_model=Exported)
def read_exported() -> Exported:
    return Exported(email="a@example.com", hashed_password="x")


@router.post("/hooks", response_model=HookCreated)
def create_hook() -> HookCreated:
    return HookCreated(id=1, webhook_secret="s")


@router.get("/mailers/{mailer_id}", response_model=Mailer)  # caught
def read_mailer(mailer_id: int, session: SessionDep):
    return session.get(Mailer, mailer_id)


@router.get("/operators/{operator_id}", response_model=Operator)  # caught
def read_operator(operator_id: int, session: SessionDep):
    return session.get(Operator, operator_id)


@router.get("/look-alike", response_model=LookAlike)  # caught
def read_look_alike() -> LookAlike:
    return LookAlike(password="x")


@router.get("/toggles/{toggle_id}", response_model=Toggles)
def read_toggles(toggle_id: int, session: SessionDep):
    return session.get(Toggles, toggle_id)


@router.get("/alias-masked", response_model=AliasMasked)
def read_alias_masked() -> AliasMasked:
    return AliasMasked(password="x")


@router.get("/annotated-masked", response_model=AnnotatedMasked)
def read_annotated_masked() -> AnnotatedMasked:
    return AnnotatedMasked(password="x")
