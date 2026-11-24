from config.settings.base import *  # noqa: F403

REST_FRAMEWORK["DEFAULT_PAGINATION_CLASS"] = "notes.pagination.NotePages"  # noqa: F405
