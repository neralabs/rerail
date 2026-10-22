class B: pass
class A:
    class B:
        class C:
            class D(B):
                def f(self): return super(A.B.C.D, self).f()
                def g(self): return super(B.C.D, self).g()
                def h(self): return super(((A.B.C.D)), (self)).h()
                def i(self): return (super)(D, self).i()
def outer():
    C = 1
    class C(B):
        def a(self):
            nonlocal C
            return super(C, self).a()
        def b(self):
            [(C := 1) for _ in range(2)]
            return super(C, self).b()
    return C
