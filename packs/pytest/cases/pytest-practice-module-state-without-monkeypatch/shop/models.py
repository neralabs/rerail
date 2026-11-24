class Manager:
    def update(self, **values):
        return 0


class Order:
    objects = Manager()
    status = "new"
