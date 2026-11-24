import json

from pydantic import TypeAdapter

from app.schemas import Order, Settings
from app.vendors import Basket


def handle_webhook(raw: bytes) -> Order:
    return Order.model_validate(json.loads(raw))


baskets = TypeAdapter(list[Basket])


def load_settings(data: dict) -> Settings:
    return Settings(**data)
