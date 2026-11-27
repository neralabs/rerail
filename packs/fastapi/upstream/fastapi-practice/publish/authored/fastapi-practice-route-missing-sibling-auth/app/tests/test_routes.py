from fastapi import APIRouter, Depends

from app.deps import get_current_user

router = APIRouter()


@router.get("/a")
def a(user=Depends(get_current_user)):
    return {}


@router.get("/b")
def b(user=Depends(get_current_user)):
    return {}


@router.post("/c")
def c(user=Depends(get_current_user)):
    return {}


@router.post("/d")
def d():
    return {}
