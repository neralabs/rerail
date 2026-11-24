from shop.helpers import recount_books


def test_recount(db):
    recount_books()
