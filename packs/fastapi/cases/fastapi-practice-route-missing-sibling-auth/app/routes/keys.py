from fastapi import APIRouter, Depends, Security

from app.deps import api_key_header, check_key

router = APIRouter(prefix="/machines")


@router.get("/")
async def list_machines(key: str = Security(api_key_header)):
    return []


@router.post("/")
async def register_machine(key=Depends(check_key)):
    return {}


@router.patch("/{machine_id}")
async def update_machine(machine_id: int, key=Depends(check_key)):
    return {}


@router.post("/{machine_id}/reboot")
async def reboot_machine(machine_id: int):  # caught
    return {}
