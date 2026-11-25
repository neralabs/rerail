import json

from django.http import JsonResponse
from fastapi import APIRouter
from fastapi import responses as fastapi_responses
from fastapi.encoders import jsonable_encoder
from fastapi.responses import JSONResponse, ORJSONResponse, UJSONResponse
from starlette.responses import JSONResponse as JR

from app import responses as app_responses
from app.lookalike import JSONResponse as LookalikeResponse
from app.responses import AppJSONResponse, EncodedResponse, FastChildResponse, FastResponse
from app.responses import JSONResponse as ReexportedJSONResponse
from app.schemas import Event, Invoice, Plain

router = APIRouter()


# ---- reported: Starlette's JSONResponse renders with json.dumps
@router.get("/event")
def get_event(e: Event):
    return JSONResponse(content=e.model_dump())  # caught


def created(e: Event):
    return JSONResponse(e.model_dump(), status_code=201)  # caught


def aliased(e: Event):
    return JR(content={"data": e.model_dump()})  # caught


def module_attribute(invoice: Invoice):
    return fastapi_responses.JSONResponse(status_code=200, content=invoice.model_dump())  # caught


def reexported(e: Event):
    return ReexportedJSONResponse(content=e.model_dump())  # caught


def subclass(e: Event):
    return AppJSONResponse(content=e.model_dump())  # caught


def subclass_of_subclass(e: Event):
    return app_responses.VersionedResponse(e.model_dump())  # caught


# ---- reported: a conditional expression, `or` and `and`
def maybe_response(e: Event | None):
    return JSONResponse(content=e.model_dump() if e else {})  # caught


def maybe_json(e: Event | None) -> str:
    return json.dumps(e.model_dump() if e else {})  # caught


def or_default(e: Event, fallback: dict) -> str:
    return json.dumps(e.model_dump() or fallback)  # caught


def and_guard(e: Event | None) -> str:
    return json.dumps(e and e.model_dump())  # caught


# ---- left alone
def orjson_response(e: Event):
    return ORJSONResponse(content=e.model_dump())


def ujson_response(e: Event):
    return UJSONResponse(e.model_dump())


def own_render(e: Event):
    return FastResponse(content=e.model_dump())


def inherits_own_render(e: Event):
    return FastChildResponse(content=e.model_dump())


def converts_in_init(e: Event):
    return EncodedResponse(content=e.model_dump())


def json_mode_response(e: Event):
    return JSONResponse(content=e.model_dump(mode="json"))


def encoded_response(e: Event):
    return JSONResponse(content=jsonable_encoder(e.model_dump()))


def json_native_response(plain: Plain):
    return JSONResponse(plain.model_dump())


def django_response(e: Event):
    return JsonResponse(e.model_dump())


def lookalike_response(e: Event):
    return LookalikeResponse(e.model_dump())


def dump_as_header(e: Event):
    return JSONResponse(content={"ok": True}, headers={"x-event": str(e.model_dump())})
