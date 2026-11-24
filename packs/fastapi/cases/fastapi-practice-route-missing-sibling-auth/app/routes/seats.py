from fastapi import APIRouter
from pydantic import BaseModel

from app.deps import CurrentUser

router = APIRouter(prefix="/seats")


class SeatClaimBase(BaseModel):
    invitation_token: str


class SeatClaim(SeatClaimBase):
    note: str = ""


class SeatAssign(BaseModel):
    email: str
    country_code: str


@router.post("/")
async def assign_seat(body: SeatAssign, user: CurrentUser):
    return {}


@router.delete("/{seat_id}")
async def revoke_seat(seat_id: int, user: CurrentUser):
    return {}


@router.get("/")
async def list_seats(user: CurrentUser):
    return []


# The invitation token in the body is the credential (polar's claim_seat): checked by hand.
@router.post("/claim")
async def claim_seat(seat_claim: SeatClaim):
    return {}


shares = APIRouter(prefix="/shares")


@shares.get("/")
async def list_shares(user: CurrentUser):
    return []


@shares.post("/")
async def create_share(user: CurrentUser):
    return {}


@shares.delete("/{share_id}")
async def delete_share(share_id: int, user: CurrentUser):
    return {}


# A share token in the query string is the credential.
@shares.post("/{share_id}/comments")
async def comment_on_share(share_id: int, body: dict, token: str):
    return {}


teams = APIRouter(prefix="/teams")


@teams.get("/")
async def list_teams(user: CurrentUser):
    return []


@teams.post("/")
async def create_team(user: CurrentUser):
    return {}


@teams.delete("/{team_id}")
async def delete_team(team_id: int, user: CurrentUser):
    return {}


# A body whose only code-named field is a country code carries no credential.
@teams.post("/{team_id}/members")
async def add_member(team_id: int, body: SeatAssign):  # caught
    return {}
