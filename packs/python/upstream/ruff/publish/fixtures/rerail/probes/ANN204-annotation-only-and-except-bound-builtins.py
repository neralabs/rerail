staticmethod: type

try:
    pass
except Exception as classmethod:
    pass


class A:
    @staticmethod
    def __new__(cls): ...
    @classmethod
    def __init_subclass__(cls): ...
    def __init__(self): ...
