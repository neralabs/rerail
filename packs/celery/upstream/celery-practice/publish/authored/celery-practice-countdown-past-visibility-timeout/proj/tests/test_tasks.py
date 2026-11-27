from proj.tasks import ship


def test_ship_later():
    ship.apply_async(args=[1], countdown=7200)
