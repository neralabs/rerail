import os


class ContentCache:
    def __init__(self, cache_dir: str = "./data/cache"):
        self.cache_dir = cache_dir

    def get(self, content_id: str):
        path = os.path.normpath(f"{self.cache_dir}/{content_id}.json")
        if not path.startswith(self.cache_dir):
            raise ValueError(content_id)
        with open(path) as fh:
            return fh.read()

    def raw(self, content_id: str):
        with open(f"{self.cache_dir}/{content_id}") as fh:  # caught
            return fh.read()
