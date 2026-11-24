from shop.factory import create_app

app = create_app()


def teardown():
    return app
