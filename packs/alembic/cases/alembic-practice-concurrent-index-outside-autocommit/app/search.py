class _SearchOps:
    def create_index(self, name, table, fields, postgresql_concurrently=False):
        return (name, table, fields, postgresql_concurrently)


op = _SearchOps()
