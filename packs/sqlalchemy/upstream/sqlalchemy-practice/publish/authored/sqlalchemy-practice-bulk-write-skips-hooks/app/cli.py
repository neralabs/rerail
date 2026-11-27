from alembic import command as alembic_command
from alembic.config import Config
from sqlalchemy import update
from sqlalchemy.orm import Session

from app.models import Item


def upgrade_database():
    # runs Alembic, but is no migration itself
    alembic_command.upgrade(Config("alembic.ini"), "head")


def reset_labels(session: Session):
    session.execute(update(Item).values(label=""))  # caught
