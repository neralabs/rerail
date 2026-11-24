from pydantic import BaseModel, root_validator, validator


class LegacyUser(BaseModel):
    email: str
    tags: list[str]
    password: str
    password2: str

    @validator("email")
    def email_lower(cls, v):  # caught
        if v != v.lower():
            raise ValueError("email must be lowercase")

    @validator("tags", each_item=True, pre=True)
    def tag_strip(cls, v):
        if not isinstance(v, str):
            return  # caught
        return v.strip()

    @root_validator(pre=True)
    def fill_password2(cls, values):  # caught
        values.setdefault("password2", values.get("password"))

    @root_validator
    def passwords_match(cls, values):  # caught
        if values.get("password") != values.get("password2"):
            raise ValueError("passwords differ")

    @root_validator(skip_on_failure=True)
    def passwords_ok(cls, values):
        if values.get("password") != values.get("password2"):
            raise ValueError("passwords differ")
        return values

    @validator("password")
    def password_ok(cls, v):
        if len(v) < 8:
            raise ValueError("too short")
        return v
