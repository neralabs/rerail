try:
    pass
except ():
    pass

try:
    pass
except ():  # noqa
    pass

try:
    pass
except ():  # noqa: B029
    pass

try:
    pass
except ():  # NOQA: B029
    pass

try:
    pass
except ():  # noqa: E501
    pass
