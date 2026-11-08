import datetime as dt
from datetime import datetime, timezone
from zoneinfo import ZoneInfo


def stamp():
    return datetime.now()


def stamp_module():
    return dt.datetime.now()


def stamp_utc():
    return datetime.now(timezone.utc)


def stamp_paris():
    return datetime.now(tz=ZoneInfo("Europe/Paris"))
