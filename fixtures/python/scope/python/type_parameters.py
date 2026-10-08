type RecordCallback[R: Record] = list[R]
type Alias = Later
type Constrained[C: (Record, Later)] = C


def bounded[T: Later](value: T) -> T:
    return value


class Box[B: Later]:
    pass


class Record:
    pass


class Later:
    pass


def in_a_function():
    type Local = LocalLater
    LocalLater = int
    return Local


type Undefined = NeverDefined
