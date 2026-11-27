from unittest import mock


# The test imports shop.worker inside the patch, so the copy it binds is the mock.
def test_teardown():
    with mock.patch("shop.factory.create_app", return_value=object()):
        from shop.worker import teardown
    teardown()


def test_other():
    from shop.worker import teardown

    teardown()


# Another test imported shop.worker in its body; this one does not, so the patch misses its copy.
@mock.patch("shop.factory.create_app")  # caught
def test_patch_elsewhere(create_app):
    pass
