import os


def test_covered_by_autouse_restore():
    os.environ["SHOP_ISOLATED"] = "1"
    os.environ.pop("HOME", None)
