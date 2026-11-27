"""two renames on one table, rendered as two adds and two drops; a helper does a third table's"""
import sqlalchemy as sa
from alembic import op

revision = "a4"
down_revision = "a3"
branch_labels = None
depends_on = None


def _rename_project_columns():
    op.add_column("projects", sa.Column("owner_uuid", sa.Uuid()))
    op.drop_column("projects", "owner_guid")  # caught


def upgrade():
    op.add_column("members", sa.Column("email_address", sa.String(255), nullable=True))
    op.add_column("members", sa.Column("login_count", sa.Integer(), nullable=True))
    op.drop_column("members", "email")  # caught
    op.drop_column("members", "logins")  # caught
    _rename_project_columns()


def downgrade():
    op.add_column("projects", sa.Column("owner_guid", sa.UUID()))
    op.add_column("members", sa.Column("logins", sa.INTEGER(), nullable=True))
    op.add_column("members", sa.Column("email", sa.VARCHAR(length=255), nullable=True))
    op.drop_column("members", "login_count")
    op.drop_column("members", "email_address")
    op.drop_column("projects", "owner_uuid")
