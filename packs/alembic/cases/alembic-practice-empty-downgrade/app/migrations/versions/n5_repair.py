"""restore event_sequences where an old deploy dropped it"""
import sqlalchemy as sa
from alembic import op

revision = "n5"
down_revision = "n4"
branch_labels = None
depends_on = None


def upgrade():
    inspector = sa.inspect(op.get_bind())
    if not inspector.has_table("event_sequences"):
        op.create_table("event_sequences", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
