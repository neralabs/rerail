try:
    pass
except 1:
    pass

try:
    pass
except 1:  # noqa
    pass

try:
    pass
except 1:  # noqa: B030
    pass

try:
    pass
except 1:  # NOQA: B030
    pass

try:
    pass
except 1:  # noqa: E501
    pass
