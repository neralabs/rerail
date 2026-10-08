class Base:
    def __init__(self, value):
        self.value = value
        self.setup()

    def setup(self):
        return self.describe()

    def describe(self):
        return "base"

    @staticmethod
    def make(value):
        return Base(value)

    @classmethod
    def build(cls, value):
        return cls(value)

    @property
    def doubled(self):
        return self.value * 2


class Child(Base):
    def __init__(self, value):
        super().__init__(value)

    def describe(self):
        return "child " + super().describe()


class NoInit:
    def go(self):
        return self.helper()

    def helper(self):
        return 1


class Inherits(NoInit):
    pass


class Callable:
    def __call__(self, x):
        return x


def exercise():
    b = Base(1)
    c = Child(2)
    Base.make(3)
    Base.build(4)
    Child.build(5)
    b.describe()
    Base.describe(c)
    NoInit().go()
    Inherits().go()
    Callable()(6)
    return b.doubled
