from pydantic import BaseModel, TypeAdapter, field_validator, model_validator


class Line(BaseModel):
    sku: str

    # Reached through Basket.lines: Basket is parsed by a TypeAdapter.
    @field_validator("sku", mode="before")
    @classmethod
    def sku_upper(cls, v):
        return v.upper()  # caught


class Basket(BaseModel):
    lines: list[Line]


# Built only from the project's own values (keyword arguments), never parsed from foreign data: left alone.
class VectorConfig(BaseModel):
    host: str
    port: int

    @model_validator(mode="before")
    @classmethod
    def validate_config(cls, values):
        if not values["host"]:
            raise ValueError("config VECTOR_HOST is required")
        return values


def make_config(host: str, port: int) -> VectorConfig:
    return VectorConfig(host=host, port=port)


# A value test that raises leaves only the expected string: left alone.
class Quote(BaseModel):
    currency: str

    @field_validator("currency", mode="before")
    @classmethod
    def same_currency(cls, value):
        if value != "EUR":
            raise ValueError("only EUR")
        return value.upper()


quotes = TypeAdapter(Quote)


# Parsed only in a test: left alone.
class Draft(BaseModel):
    title: str

    @field_validator("title", mode="before")
    @classmethod
    def strip_title(cls, v):
        return v.strip()


# Parsed from the project's own values: an object read by attribute, a dict literal, a name bound to one.
class WorkflowOut(BaseModel):
    env: list[str]

    @field_validator("env", mode="before")
    @classmethod
    def env_names(cls, value):
        return [item.name for item in value]


class EsConfig(BaseModel):
    host: str
    port: int = 9200

    @model_validator(mode="before")
    @classmethod
    def default_port(cls, values):
        values.setdefault("port", 9200)
        return values


def to_out(workflow) -> WorkflowOut:
    return WorkflowOut.model_validate(workflow, from_attributes=True)


def es_config(host: str) -> EsConfig:
    config = {"host": host}
    return EsConfig.model_validate(config)


def es_default() -> EsConfig:
    return EsConfig.model_validate({"host": "localhost"})


def normalise_input(method):
    def wrapper(cls, data, info):
        return method(cls, data if isinstance(data, dict) else {"value": data})

    return wrapper


# A project wrapper between Pydantic and the def decides what the def receives: left alone.
class Tagged(BaseModel):
    value: str = ""

    @model_validator(mode="before")
    @classmethod
    @normalise_input
    def drop_legacy(cls, data):
        data.pop("legacy", None)
        return data


tagged = TypeAdapter(list[Tagged])
