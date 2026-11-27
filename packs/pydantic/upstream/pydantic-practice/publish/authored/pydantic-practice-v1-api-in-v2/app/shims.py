import importlib.metadata

import pydantic as pd
from fastapi._compat import PYDANTIC_V2
from pydantic import VERSION as PV

from app import compat
from app.compat import API_V1_STR, MAJOR
from app.compat import PYDANTIC_V2 as IS_V2
from app.schemas import Account, User


def by_version(acct: Account):
    if PV.startswith("1."):
        return acct.json(indent=2)
    return acct.model_dump_json(indent=2)


def by_module_version(acct: Account):
    if pd.VERSION[0] == "1":
        return acct.json(sort_keys=True)
    return acct.model_dump_json()


def by_imported_flag(acct: Account):
    if not IS_V2:
        return acct.json(indent=2)
    return acct.model_dump_json(indent=2)


def by_module_flag(acct: Account):
    if compat.IS_V1:
        return acct.json(indent=2)
    return acct.model_dump_json(indent=2)


def by_major(acct: Account):
    if MAJOR >= 2:
        return acct.model_dump_json(indent=2)
    else:
        return acct.json(indent=2)


def by_library_flag(acct: Account):
    if not PYDANTIC_V2:
        return acct.json(indent=2)
    return acct.model_dump_json(indent=2)


def by_distribution(acct: Account):
    if importlib.metadata.version("pydantic").startswith("1"):
        return acct.json(indent=2)
    return acct.model_dump_json(indent=2)


def by_probe(acct: Account):
    return acct.model_dump_json(indent=2) if hasattr(acct, "model_dump_json") else acct.json(indent=2)


def by_getattr(row):
    if getattr(User, "model_validate", None) is None:
        return User.from_orm(row)
    return User.model_validate(row)


def by_elif(acct: Account, fmt: str):
    if fmt == "short":
        return acct.model_dump_json()
    elif not IS_V2:
        return acct.json(indent=2)
    return acct.model_dump_json(indent=2)


def by_local_flag(acct: Account):
    v2 = hasattr(acct, "model_dump_json")
    if not v2:
        return acct.json(indent=2)
    return acct.model_dump_json(indent=2)


def by_import(acct: Account):
    try:
        from pydantic import model_serializer  # noqa: F401
    except ImportError:
        return acct.json(indent=2)
    return acct.model_dump_json(indent=2)


def by_api_path(path: str, acct: Account):
    if path.startswith(API_V1_STR):
        return acct.json(indent=2)  # caught
    return acct.model_dump_json(indent=2)


def by_api_version(acct: Account, version: int):
    if version >= 2:
        return acct.json(sort_keys=True)  # caught
    return acct.model_dump_json()


def by_v1_flag(acct: Account, use_v1_format: bool):
    return acct.json(indent=2) if use_v1_format else acct.model_dump_json()  # caught
