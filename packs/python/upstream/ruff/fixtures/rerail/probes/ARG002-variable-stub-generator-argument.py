class C:
    def a(self, x):
        self.msg = "not implemented"
        raise NotImplementedError(i for i in ())

    def b(self, x):
        msg = "not implemented"
        raise NotImplementedError(i for i in ())

    def c(self, x):
        self.msg = "not implemented"
        raise NotImplementedError(x for x in ())
