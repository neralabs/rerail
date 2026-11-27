from typing import Annotated, Optional

from pydantic import Field, PlainSerializer, SecretStr

MaskedStr = Annotated[str, PlainSerializer(lambda v: "*****" if v else v, return_type=str)]

# Aliases of a plain string: read through, an alias of an alias included.
PlainStr = str
NonEmptyStr = Annotated[str, Field(min_length=1)]
OptionalText = Optional[PlainStr]


class VaultSecret(SecretStr):
    pass
