"""a revision whose descendants never add orders.user_id back; a1's create_table is an ancestor"""
import sqlalchemy as sa
from alembic import op

revision = "a7"
down_revision = "a6"
branch_labels = None


class _Note(sa.orm.declarative_base()):
    # A model local to the migration: not the application's.
    __tablename__ = "notes"
    id = sa.Column(sa.Integer, primary_key=True)
    body = sa.Column(sa.Text)


def upgrade():
    op.drop_column("orders", "user_id")  # caught
    op.drop_column("notes", "body")  # mapped only by the migration's own model


def downgrade():
    op.add_column("orders", sa.Column("user_id", sa.Integer()))
