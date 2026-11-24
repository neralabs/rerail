"""accounts: dialect branches"""
import sqlalchemy as sa
from alembic import op

revision = "c7"
down_revision = "c6"
branch_labels = None
depends_on = None


def upgrade():
    conn = op.get_bind()
    if conn.dialect.name == "sqlite":
        with op.batch_alter_table("accounts") as batch_op:
            batch_op.alter_column("plan", existing_type=sa.String(20), nullable=False)
    else:
        op.alter_column("accounts", "plan", existing_type=sa.String(20), nullable=False)  # caught
    op.drop_constraint("uq_accounts_slug", "accounts", type_="unique")


def downgrade():
    pass
