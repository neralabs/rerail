"""accounts service: its own users table, which no model of this tree maps"""
from alembic import op

revision = "c1"
down_revision = None


def upgrade():
    op.drop_column("users", "nick")  # the app and billing trees' models map users.nick; no model of this tree does
    op.drop_column("profiles", "bio")  # excluded from Profile's mapper
    op.drop_column("profiles", "avatar")  # caught
    op.drop_column("prefs", "theme")  # excluded as a Column object, in dict(...)
    op.drop_column("sessions", "ip")  # include_properties leaves it out
    op.drop_column("sessions", "token")  # caught
    op.drop_column("accounts", "old_flag")  # excluded by the mixin's __mapper_args__
    op.drop_column("accounts", "plan")  # caught
    op.drop_column("archived", "old_flag")  # caught
    op.drop_column("archived", "note")  # excluded by the class's own __mapper_args__
    # Gap: a Flask-SQLAlchemy model without __tablename__ (Widget, table name generated as widget) still maps color.
    op.drop_column("widget", "color")
    # Gap: the attribute was removed from the model in this same change, so the models agree with the migration; old
    # processes still query the column until the new code is live.
    op.drop_column("accounts", "legacy")


def downgrade():
    pass
