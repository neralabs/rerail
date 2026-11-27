from datetime import datetime
from uuid import UUID, uuid4

from pydantic import BaseModel


# A model defined in a test: the rule skips test code.
class FakeOrder(BaseModel):
    id: UUID = uuid4()
    created_at: datetime = datetime.now()


def test_defaults():
    started = datetime.now()
    assert FakeOrder().created_at <= started
