from fastapi import APIRouter, FastAPI

from app.versioning import version

router = APIRouter(prefix="/users")
admin = APIRouter(prefix="/admin/users")


# The documented fix: the literal route is declared first.
@router.get("/me")
def read_me():
    return {}


@router.get("/{user_id}")
def read_user(user_id: int):
    return {}


# Another router: its routes do not meet router's.
@admin.get("/me")
def admin_read_me():
    return {}


# Versioned duplicates (a route class picks one by the request's version): an extra decorator, or another name=.
@router.post("/")
def create_user():
    return {}


@router.post("/", name="create")
@version(starting_from="2027-01")
def create_user_v2027():
    return {}


@router.patch("/{user_id}")
def update_user(user_id: int):
    return {}


@router.patch("/{user_id}", name="update_v2")
def update_user_v2(user_id: int):
    return {}


@router.delete("/{user_id}", api_version=1)
def delete_user(user_id: int):
    return {}


@router.delete("/{user_id}", api_version=2)
def delete_user_v2(user_id: int):
    return {}


# One function under two paths.
@router.get("/{user_id}/avatar")
@router.get("/{user_id}/picture")
def avatar(user_id: int):
    return {}


# Registered on one branch or the other, never both.
def setup(app: FastAPI, legacy: bool):
    if legacy:
        @app.get("/home")
        def home_legacy():
            return {}
    else:
        @app.get("/home")
        def home():
            return {}

    @app.get("/{page}")
    def page(page: str):
        return {}

    @app.get("/about")  # caught
    def about():
        return {}


# A path that is not a literal string.
PREFIX = "/feeds"


@router.get(PREFIX + "/{feed}")
def feed(feed: str):
    return {}


@router.get(PREFIX + "/latest")
def latest_feed():
    return {}


# A literal brace that is not a parameter.
@router.get("/raw/{")
def raw_brace():
    return {}
