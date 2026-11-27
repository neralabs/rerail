from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Session

from .db import SessionLocal


def send_report(db: Session, user_id: int) -> None:
    db.execute("UPDATE reports SET sent = true WHERE user_id = :id", {"id": user_id})
    db.commit()


async def refresh_stats(db: AsyncSession, user_id: int) -> None:
    await db.execute("UPDATE stats SET dirty = false WHERE user_id = :id", {"id": user_id})
    await db.commit()


def send_report_by_id(user_id: int) -> None:
    with SessionLocal() as db:
        send_report(db, user_id)


def send_and_close(db: Session, user_id: int) -> None:
    try:
        send_report(db, user_id)
    finally:
        db.close()


def send_email(address: str) -> None:
    print(address)


class ReportService:
    def __init__(self, db: Session) -> None:
        self.db = db

    def run(self, user_id: int) -> None:
        send_report(self.db, user_id)
