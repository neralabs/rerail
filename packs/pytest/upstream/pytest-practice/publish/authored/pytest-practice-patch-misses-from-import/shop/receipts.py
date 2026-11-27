from shop.http import Session


def describe(session: Session) -> str:
    return repr(session)
