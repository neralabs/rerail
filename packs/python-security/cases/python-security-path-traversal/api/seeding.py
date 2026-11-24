from pathlib import Path

from api.schemas import Registration

RESOURCES = Path(__file__).parent / "resources"


def seed(registration: Registration):
    foods = (RESOURCES / "foods" / f"{registration.locale}.json").read_text()
    theme = (RESOURCES / "themes" / f"{registration.theme}.css").read_text()
    avatar = (RESOURCES / "avatars" / registration.avatar).read_bytes()  # caught
    return foods, theme, avatar
