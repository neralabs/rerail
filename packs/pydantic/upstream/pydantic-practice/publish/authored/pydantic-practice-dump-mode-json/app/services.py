import json
from datetime import datetime
from typing import Any

import orjson
from fastapi.encoders import jsonable_encoder
from flask import jsonify
from pydantic import BaseModel

from app import schemas
from app.schemas import (
    AnnotatedSerialized,
    Batch,
    Computed,
    Credentials,
    Encoders,
    Event,
    Hidden,
    Invoice,
    JsonOnlySerialized,
    LegacyEvent,
    Link,
    Plain,
    Serialized,
    Shape,
    ShapeValues,
    Tagged,
    User,
    WholeSerialized,
)
from app.utils import json as project_json


# ---- reported: a dump in Python mode, of a model with a field json cannot encode, handed to json.dumps/json.dump
def event_json(event: Event) -> str:
    return json.dumps(event.model_dump())  # caught


def write_event(event: Event, fp) -> None:
    json.dump(event.model_dump(), fp, indent=2)  # caught


def invoice_json(data: dict) -> str:
    invoice = Invoice.model_validate(data)
    payload = invoice.model_dump(by_alias=True)  # caught
    return json.dumps(payload)


def user_envelope(user: User) -> str:
    return json.dumps({"data": user.model_dump(), "ok": True})  # caught


def events_json(events: list[Event]) -> str:
    return json.dumps([e.model_dump() for e in events])  # caught


def legacy_dict(user: User) -> str:
    return json.dumps(user.dict())  # caught


def profile_json(profile: schemas.Profile) -> str:
    return json.dumps(profile.model_dump(exclude_none=True))  # caught


def batch_json(batch: Batch) -> str:
    return json.dumps(batch.model_dump())  # caught


def tagged_json() -> str:
    return json.dumps(Tagged(tags={"a", "b"}).model_dump())  # caught


def shape_json(shape: Shape) -> str:
    return json.dumps(shape.model_dump())  # caught


def link_json(link: Link) -> str:
    return json.dumps(link.model_dump())  # caught


def credentials_json(creds: Credentials) -> str:
    return json.dumps(creds.model_dump())  # caught


def json_only_serializer(item: JsonOnlySerialized) -> str:
    return json.dumps(item.model_dump())  # caught


def computed_json(item: Computed) -> str:
    return json.dumps(item.model_dump())  # caught


def explicit_python_mode(event: Event) -> str:
    return json.dumps(event.model_dump(mode="python"))  # caught


def merged(event: Event) -> str:
    return json.dumps({**event.model_dump(), "kind": "event"})  # caught


class Job(BaseModel):
    run_at: datetime

    def to_json(self) -> str:
        return json.dumps(self.model_dump(), sort_keys=True)  # caught


# ---- left alone
def json_mode(event: Event) -> str:
    return json.dumps(event.model_dump(mode="json"))


def dump_json(event: Event) -> str:
    return event.model_dump_json()


def with_default(event: Event) -> str:
    return json.dumps(event.model_dump(), default=str)


class Encoder(json.JSONEncoder):
    def default(self, o):
        return str(o)


def with_cls(event: Event) -> str:
    return json.dumps(event.model_dump(), cls=Encoder)


def with_kwargs(event: Event, **kwargs: Any) -> str:
    return json.dumps(event.model_dump(), **kwargs)


def with_orjson(event: Event) -> bytes:
    return orjson.dumps(event.model_dump())


def with_jsonable_encoder(event: Event) -> str:
    return json.dumps(jsonable_encoder(event.model_dump()))


def with_jsonify(event: Event):
    return jsonify(event.model_dump())


def with_project_json(event: Event) -> str:
    return project_json.dumps(event.model_dump())


def json_native(plain: Plain) -> str:
    return json.dumps(plain.model_dump())


def included(event: Event) -> str:
    return json.dumps(event.model_dump(include={"name"}))


def excluded(event: Event) -> str:
    return json.dumps(event.model_dump(exclude={"starts_at"}))


def unknown_model(obj) -> str:
    return json.dumps(obj.model_dump())


def any_model(obj: Any) -> str:
    return json.dumps(obj.model_dump())


def v1_model(event: LegacyEvent) -> str:
    return json.dumps(event.dict())


def field_serializer(item: Serialized) -> str:
    return json.dumps(item.model_dump())


def annotated_serializer(item: AnnotatedSerialized) -> str:
    return json.dumps(item.model_dump())


def json_encoders(item: Encoders) -> str:
    return json.dumps(item.model_dump())


def model_serializer(item: WholeSerialized) -> str:
    return json.dumps(item.model_dump())


def excluded_field(item: Hidden) -> str:
    return json.dumps(item.model_dump())


def enum_values(shape: ShapeValues) -> str:
    return json.dumps(shape.model_dump())


def overridden(item: schemas.Override) -> str:
    return json.dumps(item.model_dump())


def converted_before_encoding(event: Event) -> str:
    payload = event.model_dump()
    payload["starts_at"] = payload["starts_at"].isoformat()
    return json.dumps(payload)


def rebound_before_encoding(event: Event) -> str:
    payload = event.model_dump()
    payload = {k: str(v) for k, v in payload.items()}
    return json.dumps(payload)


def not_encoded(event: Event) -> dict:
    data = event.model_dump()
    return data


# ---- reported: the loop variable over a parameter annotated as a list of models
def publish_all(events: list[Event], fp) -> None:
    for event in events:
        fp.write(json.dumps(event.model_dump()) + "\n")  # caught


# ---- left alone: the loop variable over a collection of JSON-native models, or of unknown items
def publish_plain(items: list[Plain], fp) -> None:
    for item in items:
        fp.write(json.dumps(item.model_dump()) + "\n")


def publish_unknown(items, fp) -> None:
    for item in items:
        fp.write(json.dumps(item.model_dump()) + "\n")


# ---- reported: a before validator does not change the stored type
def parsed_first(item: schemas.ParsedFirst) -> str:
    return json.dumps(item.model_dump())  # caught


# ---- left alone: a validator replaces the value after validation (`self.host = str(self.host)`, an after
# field validator returning a str)
def syslog_json(syslog: schemas.Syslog) -> str:
    return json.dumps(syslog.model_dump())


def stamped_json(item: schemas.Stamped) -> str:
    return json.dumps(item.model_dump())
