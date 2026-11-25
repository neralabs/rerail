"""billing: its own models are the ones that count"""
from alembic import op

revision = "b2"
down_revision = "b1"


def upgrade():
    op.drop_column("invoices", "memo")  # caught
    op.drop_column("users", "wallet_id")  # caught
    op.drop_column("users", "display_name")  # only the app tree's User maps it: another service's model
    op.drop_table("hero")  # only the app tree maps hero


def downgrade():
    pass
