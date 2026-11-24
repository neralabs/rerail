from tagflow import tag, text


def greeting(name: str):
    with tag.p():
        text(f"Hello {name}")


def label(count: int, unit: str):
    text("{} {}".format(count, unit))
