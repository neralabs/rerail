from shop.models import Author


def test_seed():
    Author.objects.get_or_create(name="n")
