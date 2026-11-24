from sqlalchemy import select


def text(value: str) -> str:
    return value.strip()


def caption(name: str) -> str:
    return text(f"Recipe {name}")
