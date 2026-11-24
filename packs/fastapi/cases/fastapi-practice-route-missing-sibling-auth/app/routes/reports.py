from fastapi import APIRouter

from app.deps import CurrentUser

# main.py includes this router with `dependencies=[Depends(get_current_user)]`: every route has auth.
router = APIRouter()


@router.get("/reports")
def list_reports(user: CurrentUser):
    return []


@router.post("/reports")
def create_report(user: CurrentUser):
    return {}


@router.put("/reports/{i}")
def update_report(i: int, user: CurrentUser):
    return {}


@router.delete("/reports/{i}")
def delete_report(i: int):
    return {}
