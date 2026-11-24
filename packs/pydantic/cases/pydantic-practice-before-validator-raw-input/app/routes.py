from fastapi import APIRouter

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
