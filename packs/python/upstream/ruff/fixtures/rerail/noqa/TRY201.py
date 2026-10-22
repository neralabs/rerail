try:
    pass
except ValueError as e:
    raise e

try:
    pass
except ValueError as e:
    raise e  # noqa

try:
    pass
except ValueError as e:
    raise e  # noqa: TRY201

try:
    pass
except ValueError as e:
    raise e  # NOQA: TRY201

try:
    pass
except ValueError as e:
    raise e  # noqa: E501
