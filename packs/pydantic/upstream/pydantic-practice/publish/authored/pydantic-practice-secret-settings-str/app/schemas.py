from pydantic import BaseModel


# Request and response models are input and output, not configuration: out of scope here.
class LoginBody(BaseModel):
    username: str
    password: str


class ApiKeyCreate(BaseModel):
    name: str
    api_key: str
