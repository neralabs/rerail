from fastapi import APIRouter, Depends

from app.deps import load_owner

router = APIRouter()


@router.get("/orders")
def list_orders(owner=Depends(load_owner)):
    return []


@router.get("/orders/{order_id}")
def read_order(order_id: int, owner=Depends(load_owner)):
    return {}


@router.post("/orders")
def create_order(owner=Depends(load_owner)):
    return {}


def cancel_order(order_id: int):  # caught
    return {}


router.add_api_route("/orders/{order_id}/cancel", cancel_order, methods=["POST"])
