import requests


def get_client():
    s = requests.Session()
    s.headers["User-Agent"] = "bookmarks"
    return s
