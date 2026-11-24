from alembic import context

from app.models import Base, User  # env.py imports the models for autogenerate on purpose

target_metadata = Base.metadata
_ = User


def run_migrations_online():
    context.configure(target_metadata=target_metadata)
