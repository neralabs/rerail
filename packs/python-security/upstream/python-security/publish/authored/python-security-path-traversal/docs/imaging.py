class Image:
    """A project class that happens to share PIL's names."""

    @staticmethod
    def open(name):
        return name.upper()
