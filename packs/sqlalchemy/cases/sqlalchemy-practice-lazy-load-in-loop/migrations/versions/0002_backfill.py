from alembic import op
from sqlalchemy import select
from sqlalchemy.orm import Session

from app.models.shop import Order

revision = "0002"
down_revision = "0001"


def upgrade():
    session = Session(bind=op.get_bind())
    for order in session.scalars(select(Order)):
        order.total = len(order.tags)
    session.commit()
