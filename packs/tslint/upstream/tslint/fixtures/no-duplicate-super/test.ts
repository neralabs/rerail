declare const b: boolean;
declare const b2: boolean;
declare const n: number;

// Simple
{
    class A {
        constructor() {
            super();
            super();
        }
    }

    class B {
        constructor() {
            super();
            super.foo();
        }
    }

    class C1 {
        constructor() {
            super();

            class C2 {
                constructor() {
                    super();
                }
            }
        }
    }

    class D {
        constructor() {
            super(super());
        }
    }
}

// If/else
{
    class A {
        constructor() {
            if (b) {
                super();
                return;
            }
            super();
        }
    }

    class B {
        constructor() {
            if (b) {
                super();
            } else {
                super();
            }
        }
    }

    class C {
        constructor() {
            if (b) {
                super();
                return;
            } else if (b2) {
                super();
            }
            super();
        }
    }

    class D {
        constructor() {
            if (b) {
                super();
                return;
            } else if (b) {
                super();
                return;
            }
            super();
        }
    }
}

// Loop
{
    class A {
        constructor() {
            while (b) {
                if (b) {
                    super();
                    break;
                }
            }
        }
    }

    class B {
        constructor() {
            while (b) {
                if (b) {
                    super();
                    break;
                }
                if (b2) {
                    super();
                    break;
                }
                super();
                break;
            }
        }
    }

    class C {
        constructor() {
            while (b) {
                if (b) {
                    super();
                    return;
                } else {
                    super();
                    return;
                }
            }
            super();
        }
    }

    class D {
        constructor() {
            while (b) {
                if (b) {
                    super();
                    return;
                } else {
                    super();
                    break;
                }
            }
            super();
        }
    }

    class E {
        constructor() {
            while (b) {
                if (b) {
                    super();
                    break;
                } else {
                    super();
                    return;
                }
            }
            super();
        }
    }
}

// Switch
{
    class A {
        constructor() {
            switch (n) {
                case 0:
                    super();
                case 1:
                    super();
            }
        }
    }

    class B {
        constructor() {
            switch (n) {
                case 0:
                    super();
                    break;
                case 1:
                    super();
                    break;
            }
        }
    }

    class C {
        constructor() {
            switch (n) {
                case 0:
                    super();
                    break;
                case 1:
                    super();
                    break;
            }
            super();
        }
    }
}

// Wierd
{
    class A {
        constructor() {
            if (b) {
                super();
                {
                    return;
                }
            }
            super();
        }
    }

    class B {
        constructor() {
            switch (n) {
                case 0:
                    if (b) {
                        super();
                        return;
                    } else {
                        super();
                        if (b2) return; else return;
                    }
            }
            super();
        }
    }
}

// With ternary operator
{
    class A {
        constructor(props?: any) {
            props ? super(props) : super();
        }
    }

    class B {
        constructor(props?: any) {
            props ? super(props) : super() ? super() : super();
        }
    }
}


