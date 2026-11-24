from fastapi import APIRouter, Depends

from app.deps import CurrentUser, SessionDep, get_current_user, get_pagination, require_admin

router = APIRouter(prefix="/items")


@router.get("/")
def list_items(db: SessionDep, user: CurrentUser, page=Depends(get_pagination)):
    return []


@router.get("/{item_id}")
def read_item(item_id: int, db: SessionDep, user=Depends(get_current_user)):
    return {}


@router.post("/", dependencies=[Depends(require_admin)])
def create_item(db: SessionDep):
    return {}


@router.put("/{item_id}")
async def update_item(item_id: int, user: dict = Depends(get_current_user)):
    return {}


@router.delete("/{item_id}")
def delete_item(item_id: int, db: SessionDep):  # caught
    return {}


@router.get("/health")
def health():
    return {"status": "ok"}
