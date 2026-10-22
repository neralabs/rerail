class A:
    __slots__ = ["x01", "x0y"]
class B:
    __slots__ = ["x0y", "x01"]
class C:
    __slots__ = ["x012", "x09"]
class D:
    __slots__ = {"x09": 1, "x012": 2}
class E:
    __slots__ = ["a00000001", "a0000000a"]
class F:
    __slots__ = ["0 1", "0a"]
class G:
    __slots__ = {"10", "1 0"}
class H:
    __slots__ = ["\U0001F600", "￿"]
class I:
    __slots__ = ("￿", "\U0001F600")
