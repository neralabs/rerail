from typing import Annotated

from pydantic import PlainSerializer, SecretStr

MaskedStr = Annotated[str, PlainSerializer(lambda v: "*****" if v else v, return_type=str)]


class VaultSecret(SecretStr):
    pass
