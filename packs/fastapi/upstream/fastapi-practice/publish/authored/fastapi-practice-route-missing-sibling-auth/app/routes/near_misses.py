from fastapi import APIRouter, Depends

from app.deps import CurrentUser, get_current_user

# Three routes only: too few to tell an outlier from a design.
small = APIRouter()


@small.get("/a")
def small_a(user: CurrentUser):
    return {}


@small.get("/b")
def small_b(user: CurrentUser):
    return {}


@small.post("/c")
def small_c():
    return {}


# Two routes without auth: the router mixes public and private routes on purpose.
mixed = APIRouter()


@mixed.get("/p")
def mixed_a(user: CurrentUser):
    return {}


@mixed.post("/q")
def mixed_b(user: CurrentUser):
    return {}


@mixed.put("/r")
def mixed_c(user: CurrentUser):
    return {}


@mixed.get("/catalog")
def mixed_catalog():
    return []


@mixed.post("/feedback")
def mixed_feedback(body: dict):
    return {}


# The router itself requires the user: every route has auth.
guarded = APIRouter(prefix="/guarded", dependencies=[Depends(get_current_user)])


@guarded.get("/")
def guarded_list(user: CurrentUser):
    return []


@guarded.post("/")
def guarded_create(user: CurrentUser):
    return {}


@guarded.put("/{i}")
def guarded_update(i: int, user: CurrentUser):
    return {}


@guarded.delete("/{i}")
def guarded_delete(i: int):
    return {}


# No route has auth: nothing to compare with.
plain = APIRouter()


@plain.get("/x")
def plain_a():
    return {}


@plain.get("/y")
def plain_b():
    return {}


@plain.post("/z")
def plain_c():
    return {}


@plain.delete("/w")
def plain_d():
    return {}
