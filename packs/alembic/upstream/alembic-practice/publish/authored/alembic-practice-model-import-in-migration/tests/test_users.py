from app.models.user import User


def test_user_email():
    assert User(email="a@b.c").email == "a@b.c"
