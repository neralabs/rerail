from typing import Annotated

from fastapi import Depends
from sqlalchemy.orm import Session

from .db import get_db, get_state_db

# Default scope: the dependency's exit code runs after the response and its background tasks (FastAPI 0.118+).
SessionDep = Annotated[Session, Depends(get_db)]
# Function scope: the exit code runs before the response is sent.
FnSessionDep = Annotated[Session, Depends(get_db, scope="function")]
# The HTTP middleware's session, closed once call_next returns.
StateSessionDep = Annotated[Session, Depends(get_state_db)]
