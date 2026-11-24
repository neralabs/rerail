class _SearchOps:
    def create_index(self, name, table, fields):
        return (name, table, fields)

    def delete_index(self, name):
        return name


op = _SearchOps()
