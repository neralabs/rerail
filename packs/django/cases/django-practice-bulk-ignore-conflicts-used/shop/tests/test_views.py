from shop.models import Label


def test_labels_created():
    # Test code is skipped.
    created = Label.objects.bulk_create([Label(name="a")], ignore_conflicts=True)
    assert [label.pk for label in created]
