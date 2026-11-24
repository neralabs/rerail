from app.shared.router import router


@router.get("/{item_id}")
def read_item(item_id: int):
    return {}
