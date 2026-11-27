from typing import Annotated

from fastapi import APIRouter, Depends, Query

from app.annotated import ClaimConfirm, Contains, ItemQuery, PageQuery, Tagged
from app.guarded import Checks, Envelope, Profile
from app.models import Comment
from app.schemas import Address, LegacySignup, Signup

router = APIRouter()


# Request bodies: a client chooses what reaches the validators.
@router.post("/signup")
async def signup(body: Signup) -> None: ...


@router.put("/address")
async def set_address(body: list[Address]) -> None: ...


@router.post("/legacy")
def legacy_signup(body: LegacySignup | None = None) -> None: ...


@router.post("/comments")
def add_comment(comment: Comment) -> None: ...


@router.post("/claims/confirm")
def confirm_claim(body: ClaimConfirm, tagged: Tagged, contains: Contains) -> None: ...


# The guarded validators' models are request bodies too: only their checks keep them silent.
@router.put("/profile")
def set_profile(profile: Profile, envelope: Envelope, checks: Checks) -> None: ...


# Query models: FastAPI validates each query value as its declared type before the model's validators run.
@router.get("/items")
def list_items(q: Annotated[ItemQuery, Depends()]) -> None: ...


@router.get("/pages")
def list_pages(page: Annotated[PageQuery, Query()], again: PageQuery = Depends()) -> None: ...
