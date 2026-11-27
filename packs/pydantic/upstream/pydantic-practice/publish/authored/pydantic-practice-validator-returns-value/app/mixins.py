from app.compat import field_validator, model_validator


class SlugMixin:
    """Validators shared by the models that have a slug; the models live in app/models.py."""

    @field_validator("slug")
    @classmethod
    def slug_lower(cls, v: str) -> str:  # caught
        if v != v.lower():
            raise ValueError("slug must be lowercase")

    @model_validator(mode="after")
    def slug_matches_title(self):  # caught
        if self.slug not in self.title.lower():
            raise ValueError("slug must come from the title")


class TitleMixin:
    @field_validator("title")
    @classmethod
    def title_ok(cls, v: str) -> str:
        if not v:
            raise ValueError("title required")
        return v.strip()
