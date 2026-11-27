import random
import secrets
import uuid
from datetime import date, datetime, timedelta, timezone
from datetime import datetime as dt
from uuid import UUID, uuid4

import pydantic
from pydantic import BaseModel, Field, PrivateAttr, RootModel
from sqlmodel import Field as SQLField
from sqlmodel import SQLModel


class Order(BaseModel):
    id: UUID = uuid4()  # caught
    created_at: datetime = datetime.now()  # caught
    updated_at: datetime = datetime.utcnow()  # caught
    placed_at: datetime = datetime.now(timezone.utc)  # caught
    day: date = date.today()  # caught
    ref: str = str(uuid4())  # caught
    code: str = uuid4().hex  # caught
    token: str = secrets.token_urlsafe(16)  # caught
    jitter: float = random.random()  # caught
    expires_at: datetime = dt.now() + timedelta(days=7)  # caught
    key: str = Field(uuid.uuid4().hex, max_length=32)  # caught
    issued_at: datetime = Field(default=dt.now(timezone.utc), description="when issued")  # caught
    stamp: str = Field(
        default=datetime.now().isoformat(),  # caught
        description="multi-line Field call",
    )
    _nonce: str = secrets.token_hex(8)  # caught
    _opened: datetime = PrivateAttr(default=datetime.now())  # caught
    _seed = PrivateAttr(random.randint(0, 9))  # caught


class Tick(RootModel[datetime]):
    root: datetime = datetime.now()  # caught


class Item(SQLModel, table=True):
    id: uuid.UUID = SQLField(default=uuid.uuid4(), primary_key=True)  # caught
    name: str


class Account(pydantic.BaseModel):
    opened_on: date = pydantic.Field(default=date.today())  # caught
