defined = 1
annotated_only: int


class Holder:
    in_a_class: int


def body():
    in_a_function: int


__all__ = ["defined", "annotated_only", "in_a_class", "in_a_function", "missing"]
__all__ += ["print", "__file__", "__name__", "ExceptionGroup"]
__all__ += ["missing" "7", "def" "ined", "print" f"{1}"]
__all__ += ["esc\x61ped", "defin\x65d", u"unicode_prefixed", r"raw\x"]
import builtins
import builtins as b

__all__ += builtins.list(["by_builtins_list"])
__all__ += b.tuple(("by_alias",))
__all__ += (list)(["by_parenthesized"])
__all__ += list[str](["by_subscript"])
__all__ += builtins.set(["not_a_sequence_call"])
__all__ += ["left_of_a_sum"] + (["right_nested"] + ["sum"])
__all__ += (["left_nested"] + ["sum"]) + ["right_of_a_sum"]
alias = __all__ = ["second_target"]
