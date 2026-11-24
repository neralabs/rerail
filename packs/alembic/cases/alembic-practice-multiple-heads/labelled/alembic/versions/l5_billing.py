"""billing line, declared to depend on the main line"""
import sqlalchemy as sa
from alembic import op

revision = "l5"
down_revision = "l1"
branch_labels = None
depends_on = "l2"

def upgrade():
    op.create_table("invoices", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
