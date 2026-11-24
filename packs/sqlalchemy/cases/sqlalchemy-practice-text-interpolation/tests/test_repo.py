from sqlalchemy import text


def test_raw(session, name):
    session.execute(text(f"DELETE FROM recipe WHERE name = '{name}'"))
