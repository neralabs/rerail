from django.db import IntegrityError, transaction

from shop.models import Tag


def test_duplicate():
    with transaction.atomic():
        try:
            Tag.objects.create(name="a")
        except IntegrityError:
            pass
