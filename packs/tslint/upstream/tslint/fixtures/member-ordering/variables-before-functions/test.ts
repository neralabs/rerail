// ensure that Bar and Baz do not get conflated
interface Bar {
    x(): void;
}

interface Baz {
    y: number;
}

interface BarBaz {
    x(): void;
    y: number;
}

class Foo {
    x(): void {}
    y: number;
}

interface IArrowMethods {
    a: number;
    b(): void;
    d: () => void;
}

class ArrowMethods {
    public a = 5;
    private callback: () => void
    public b = "foo";
    public c() {}
    public d = () => {};
    public e = function() {};
}

class OutOfOrder {
    a = () => {};
    b = 5;
}

class Constructor1 {
    blah: number = 0;
    constructor() {}
}

class Constructor2 {
    constructor() {}
    blah: number = 0;
}

// Works for type literal, just like interface
type T = {
    x(): void;
    y: number;
}

// Works for class inside object literal
const o = {
    foo: class C {
        x(): void;
        y: number;
    }
}

