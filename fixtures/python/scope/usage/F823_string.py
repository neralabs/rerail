T = int


def annotates_a_string():
    y: "T" = 1
    T = 1
    return y


def annotates_in_place():
    y: T = 1
    T = 1
    return y
