class Sandbox:
    def eval(self, source):
        return len(source)


def run(source):
    # A method named eval is not the builtin.
    return Sandbox().eval(source)
