class Client:
    def get(self, url):
        raise RuntimeError("real HTTP")


def fetch(url):
    raise RuntimeError("real HTTP")


def post(url):
    raise RuntimeError("real HTTP")


class Session:
    pass
