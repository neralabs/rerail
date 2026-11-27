"""tags on d1, declared to depend on d2: depends_on orders the two lines but joins no heads"""
import sqlalchemy as sa
from alembic import op

revision = "d3"
down_revision = "d1"  # caught
branch_labels = None
depends_on = "d2"


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("tags")
