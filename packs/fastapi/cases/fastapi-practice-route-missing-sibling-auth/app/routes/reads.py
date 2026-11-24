from fastapi import APIRouter

from app.deps import CurrentUser

# The one route without auth is a GET: not reported (public reads are too common to tell apart).
router = APIRouter(prefix="/wiki")


@router.post("/pages")
def create_page(user: CurrentUser):
    return {}


@router.put("/pages/{i}")
def update_page(i: int, user: CurrentUser):
    return {}


@router.delete("/pages/{i}")
def delete_page(i: int, user: CurrentUser):
    return {}


@router.get("/changelog")
def changelog():
    return []


# A write declared through api_route's methods= is reported like a decorator's verb.
admin = APIRouter()


@admin.get("/flags")
def list_flags(user: CurrentUser):
    return []


@admin.put("/flags/{name}")
def set_flag(name: str, user: CurrentUser):
    return {}


@admin.delete("/flags/{name}")
def drop_flag(name: str, user: CurrentUser):
    return {}


@admin.api_route("/flags/{name}/toggle", methods=["POST"])
def toggle_flag(name: str):  # caught
    return {}


@admin.api_route("/flags/export", methods=["GET", "HEAD"])
def export_flags(user: CurrentUser):
    return []
