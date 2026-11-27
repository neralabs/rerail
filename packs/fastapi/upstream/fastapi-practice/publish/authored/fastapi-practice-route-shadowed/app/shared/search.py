from app.shared.router import router


# The same router, another module: the order between modules is the order of their imports, which the rule does not
# know.
@router.get("/search")
def search_items():
    return []
