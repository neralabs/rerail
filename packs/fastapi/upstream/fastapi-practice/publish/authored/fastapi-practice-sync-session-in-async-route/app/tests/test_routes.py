from fastapi import APIRouter, Depends
from sqlalchemy.orm import Session

from app.crud import get_users
from app.db import get_db

router = APIRouter()


@router.get("/test-users")
async def users_in_test(db: Session = Depends(get_db)):
    db.query(object).all()
    return get_users(db)
