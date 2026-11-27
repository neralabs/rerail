import requests

from app.schemas import Account, Report, Tag, User


def load(data: dict, row):
    user = User.parse_obj(data)
    tag = Tag.from_orm(row)
    plain = User.from_orm(row)  # caught
    acct = Account.from_orm(row)
    built = Tag.construct(name="x")
    return user, tag, plain, acct, built


def render(acct: Account, report: Report, user: User):
    text = acct.json()
    pretty = acct.json(indent=2)  # caught
    ordered = acct.json(by_alias=True, sort_keys=True)  # caught
    trimmed = acct.json(exclude_none=True)
    again = user.json(indent=4)  # caught
    titled = report.json(indent=2)
    fine = acct.model_dump_json(indent=2)
    data = acct.dict()
    return text, pretty, ordered, trimmed, again, titled, fine, data


def not_models(row, settings: dict):
    resp = requests.get("https://example.com", timeout=5)
    body = resp.json()
    merged = settings.copy()
    other = row.from_orm(row)
    return body, merged, other
