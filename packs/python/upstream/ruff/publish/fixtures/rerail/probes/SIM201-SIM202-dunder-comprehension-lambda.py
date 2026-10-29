class C:
    def __ne__(self, other):
        a = [x for x in (not self == other,)]
        g = lambda y=not self == other: y
        return a, g
    def __eq__(self, other):
        a = [x for x in (not self != other,)]
        g = lambda y=not self != other: y
        return a, g
