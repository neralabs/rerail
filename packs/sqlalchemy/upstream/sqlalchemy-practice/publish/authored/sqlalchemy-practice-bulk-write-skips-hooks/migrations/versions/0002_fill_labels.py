from alembic import op
from sqlalchemy import update

from app.models import Item

revision = "0002"
down_revision = "0001"


def upgrade():
    op.get_bind().execute(update(Item).where(Item.label.is_(None)).values(label=""))


def downgrade():
    pass
