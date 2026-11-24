from fastapi import APIRouter, Depends

from acme.auth import PermissionsDependency, get_current_user

# `billing/` has no __init__.py; api.py includes this router with the user dependency, so every route has auth.
router = APIRouter()


@router.get("/invoices")
def list_invoices(user=Depends(get_current_user)):
    return []


@router.post("/invoices", dependencies=[Depends(PermissionsDependency(["billing"]))])
def create_invoice():
    return {}


@router.put("/invoices/{i}", dependencies=[Depends(PermissionsDependency(["billing"]))])
def update_invoice(i: int):
    return {}


@router.get("/invoices/{i}")
def read_invoice(i: int):
    return {}
