import json

import pydantic
import requests
from pydantic import TypeAdapter, parse_obj_as, validate_arguments
from pydantic.json import pydantic_encoder

from app.schemas import Account, Report, Rule, Tag, TitledReport, User


def create_user(body: User, db):
    row = db.add(**body.dict())  # caught
    payload = body.dict(exclude_unset=True)  # caught
    db.save(body.model_dump(exclude_unset=True))
    return row, payload


def load(data: dict, raw: str, row, path):
    user = User.parse_obj(data)  # caught
    again = User.parse_raw(raw)  # caught
    from_file = User.parse_file(path)  # caught
    tag = Tag.from_orm(row)  # caught
    plain = User.from_orm(row)
    acct = Account.model_validate(row)
    copy = acct.copy(update={"id": 2})  # caught
    built = Tag.construct(name="x")  # caught
    checked = User.validate(data)  # caught
    return user, again, from_file, tag, plain, copy, built, checked


def render(acct: Account):
    text = acct.json()  # caught
    pretty = acct.json(indent=2)
    trimmed = acct.json(exclude_none=True)  # caught
    schema = Account.schema()  # caught
    schema_text = Account.schema_json(indent=2)  # caught
    Account.update_forward_refs()  # caught
    names = list(Account.__fields__)  # caught
    touched = acct.__fields_set__  # caught
    fine = acct.model_dump_json(indent=2)
    return text, pretty, trimmed, schema, schema_text, names, touched, fine


def helpers(data):
    tags = parse_obj_as(list[Tag], data)  # caught
    more = pydantic.parse_obj_as(list[Tag], data)  # caught
    same = TypeAdapter(list[Tag]).validate_python(data)
    dumped = json.dumps(tags, default=pydantic_encoder)  # caught
    return tags, more, same, dumped


@validate_arguments  # caught
def scale(x: int, factor: int = 2) -> int:
    return x * factor


def not_models(report: Report, titled: TitledReport, rule: Rule, settings: dict):
    resp = requests.get("https://example.com", timeout=5)
    body = resp.json()
    merged = settings.copy()
    shaped = report.dict()
    inherited = titled.dict()
    ok = rule.validate("x")
    return body, merged, shaped, inherited, ok


def compat(acct: Account):
    if hasattr(acct, "model_dump"):
        data = acct.model_dump()
    else:
        data = acct.dict()
    return data


def field_names():
    try:
        return list(Account.model_fields)
    except AttributeError:
        return list(Account.__fields__)
