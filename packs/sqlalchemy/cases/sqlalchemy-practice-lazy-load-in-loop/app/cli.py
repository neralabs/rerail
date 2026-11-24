from alembic import command as alembic_command
from alembic.config import Config
from sqlalchemy.orm import Session

from app.models.shop import Order


def upgrade_database():
    # runs Alembic, but is no migration itself
    alembic_command.upgrade(Config("alembic.ini"), "head")


def report(session: Session):
    for order in session.query(Order).all():
        print(order.customer.name)  # caught
