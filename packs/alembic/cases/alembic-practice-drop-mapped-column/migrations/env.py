from alembic import context

from app.models import Base

# app.models is a package: its __init__ imports only the base; user.py and hero.py are its other modules.
target_metadata = Base.metadata
context.configure(url="postgresql://app", target_metadata=target_metadata)
