"""copy through an ORM session"""
import sqlalchemy as sa
from alembic import op

revision = "b4"
down_revision = "b3"
branch_labels = None
depends_on = None


from sqlalchemy.orm import Session


def upgrade():
    op.add_column("case", sa.Column("assignee_ref", sa.Integer()))
    session = Session(bind=op.get_bind())
    session.commit()
    op.drop_column("case", "assignee_id")


def downgrade():
    op.add_column("case", sa.Column("assignee_id", sa.INTEGER()))
    op.drop_column("case", "assignee_ref")
