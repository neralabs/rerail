def template_operand(x):
    try:
        n = 1
        return t"{x}" + t"a"
    except ValueError:
        pass


def format_string_operand(x):
    try:
        n = 1
        return "a" + f"{x}"
    except ValueError:
        pass
