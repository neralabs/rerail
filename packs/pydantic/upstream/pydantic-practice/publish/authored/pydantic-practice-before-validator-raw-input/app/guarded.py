from typing import Any

from pydantic import BaseModel, ValidationInfo, field_validator, model_validator


def clean(value: Any) -> str:
    return str(value)


class Profile(BaseModel):
    name: str
    bio: str = ""
    city: str = ""
    phone: str = ""
    lang: str = ""
    tz: str = ""
    handle: str = ""
    slug: str = ""
    color: str = ""
    label: str = ""
    kind: str = ""
    region: str = ""
    note: str = ""
    owner: str = ""

    # After mode: the value is already a str.
    @field_validator("name")
    @classmethod
    def name_after(cls, v: str) -> str:
        return v.strip()

    @field_validator("name", mode="after")
    @classmethod
    def name_after_explicit(cls, v: str) -> str:
        return v.title()

    # Early exit for other types.
    @field_validator("bio", mode="before")
    @classmethod
    def bio_early_exit(cls, v):
        if not isinstance(v, str):
            return v
        return v.strip()

    # The use inside the isinstance branch.
    @field_validator("city", mode="before")
    @classmethod
    def city_inside(cls, v):
        if isinstance(v, str):
            return v.strip()
        return v

    # A conditional expression and a boolean guard.
    @field_validator("phone", mode="before")
    @classmethod
    def phone_ternary(cls, v):
        return v.replace(" ", "") if isinstance(v, str) else v

    @field_validator("lang", mode="before")
    @classmethod
    def lang_and(cls, v):
        return isinstance(v, str) and v.lower() or v

    # Exceptions the handler catches.
    @field_validator("tz", mode="before")
    @classmethod
    def tz_try_attribute(cls, v):
        try:
            return v.upper()
        except AttributeError:
            return v

    @field_validator("handle", mode="before")
    @classmethod
    def handle_try_tuple(cls, v):
        try:
            return v["handle"]
        except (TypeError, KeyError):
            return v

    @field_validator("slug", mode="before")
    @classmethod
    def slug_try_exception(cls, v):
        try:
            return v.lower()
        except Exception as exc:
            raise ValueError("bad slug") from exc

    # hasattr, callable, type(...) is.
    @field_validator("color", mode="before")
    @classmethod
    def color_hasattr(cls, v):
        if hasattr(v, "lower"):
            return v.lower()
        return v

    @field_validator("label", mode="before")
    @classmethod
    def label_type_is(cls, v):
        if type(v) is not str:
            raise ValueError("label must be a string")
        return v.strip()

    @field_validator("kind", mode="before")
    @classmethod
    def kind_match(cls, v):
        match v:
            case str():
                return v.lower()
            case _:
                return v

    # Conversions and checks that work on any object.
    @field_validator("region", mode="before")
    @classmethod
    def region_str(cls, v):
        if v is None or v == "" or not bool(v):
            return None
        text = f"{v}"
        return str(v).strip() + text[:0] + repr(v.__class__)[:0]

    # A rebinding that converts first; calls on other names.
    @field_validator("note", mode="before")
    @classmethod
    def note_rebound(cls, v, info: ValidationInfo):
        v = clean(v)
        prefix = info.data.get("name", "")
        return cls.join(prefix, v.strip())

    @classmethod
    def join(cls, a: str, b: str) -> str:
        return a + b

    # An after model validator written as an instance method.
    @model_validator(mode="after")
    def owner_after(self):
        if not self.owner:
            self.owner = self.name.lower()
        return self


class Envelope(BaseModel):
    body: str = ""

    @model_validator(mode="before")
    @classmethod
    def unwrap(cls, data: Any) -> Any:
        if isinstance(data, dict):
            data.setdefault("body", "")
            return data.get("envelope", data)
        return data

    # An after model validator classmethod reading attributes: fine, if deprecated.
    @model_validator(mode="after")
    @classmethod
    def after_attr(cls, values):
        values.body = values.body.strip()
        return values


# The check's meaning decides, not its position.
class Checks(BaseModel):
    a: str = ""
    b: str = ""
    c: str = ""
    d: str = ""
    e: str = ""
    f: str = ""
    g: list[str] = []
    h: str = ""
    i: str = ""

    # A negated check that raises: what is left is a str.
    @field_validator("a", mode="before")
    @classmethod
    def a_raise(cls, v):
        if not isinstance(v, str):
            raise ValueError("a must be a string")
        return v.strip()

    @field_validator("b", mode="before")
    @classmethod
    def b_assert(cls, v):
        assert isinstance(v, str), "b must be a string"
        return v.strip()

    # The use in the else of a negated check, and in an elif on a positive one.
    @field_validator("c", mode="before")
    @classmethod
    def c_else(cls, v):
        if not isinstance(v, str):
            v2 = v
        else:
            return v.strip()
        return v2

    @field_validator("d", mode="before")
    @classmethod
    def d_elif(cls, v):
        if v is None:
            return ""
        elif isinstance(v, bytes):
            return v.decode()
        return v

    # `not isinstance(...) or ...`, and a conditional expression on a negated check.
    @field_validator("e", mode="before")
    @classmethod
    def e_or(cls, v):
        return not isinstance(v, str) or v.isidentifier()

    @field_validator("f", mode="before")
    @classmethod
    def f_ternary_neg(cls, v):
        return v if not isinstance(v, str) else v.strip()

    # A positive check that exits leaves every other type: an int still reaches `.split`.
    @field_validator("g", mode="before")
    @classmethod
    def g_wrong_exit(cls, v):
        if v is None or isinstance(v, list):
            return v
        return v.split(",")  # caught

    # The use on the false side of a positive check.
    @field_validator("h", mode="before")
    @classmethod
    def h_ternary_wrong(cls, v):
        return v if isinstance(v, str) else v.decode()  # caught

    # The use after an if whose body checked, but which falls through.
    @field_validator("i", mode="before")
    @classmethod
    def i_after_if(cls, v):
        if isinstance(v, str):
            v2 = v.lower()
        return v.upper()  # caught
