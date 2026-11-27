from pydantic import field_validator


# A mixin is no model, but its validators run on the models that use it.
class TrimMixin:
    @field_validator("*", mode="before")
    @classmethod
    def trim_all(cls, v):
        return v.strip()  # caught
