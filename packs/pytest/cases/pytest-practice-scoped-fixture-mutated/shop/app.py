class App:
    def __init__(self):
        self.config = {}
        self.extensions = {}
        self.debug = False
        self.counter = 0


def create_app():
    return App()
