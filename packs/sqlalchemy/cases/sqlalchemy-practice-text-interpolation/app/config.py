import os

RECIPE_TABLE = "recipes"
PAGE_SIZE = 50
SCHEMA = os.environ.get("APP_SCHEMA", "public")

INDEX_METHOD = os.environ.get("APP_INDEX_METHOD", "").strip().lower()
if INDEX_METHOD not in ("ivfflat", "hnsw", ""):
    INDEX_METHOD = ""

IVFFLAT_LISTS = os.environ.get("APP_IVFFLAT_LISTS", 100)
try:
    IVFFLAT_LISTS = int(IVFFLAT_LISTS)
except ValueError:
    IVFFLAT_LISTS = 100
