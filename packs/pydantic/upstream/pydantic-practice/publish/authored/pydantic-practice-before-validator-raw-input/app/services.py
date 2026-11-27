import json

from pydantic import TypeAdapter

from app.schemas import Order, Settings
from app.vendors import Basket, Quote, Tagged

# Built here, used from app/sources.py.
baskets = TypeAdapter(list[Basket])


def handle_webhook(raw: bytes) -> Order:
    return Order.model_validate(json.loads(raw))


def load_settings(data: dict) -> Settings:
    return Settings(**data)


def load_quote(raw: str) -> Quote:
    return TypeAdapter(Quote).validate_json(raw)


def load_tagged(raw: str) -> list[Tagged]:
    tagged = TypeAdapter(list[Tagged])
    return tagged.validate_python(json.loads(raw))
