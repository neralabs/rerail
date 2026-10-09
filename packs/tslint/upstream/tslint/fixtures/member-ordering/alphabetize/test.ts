class X {
    // Different categories have alphabetization together.
    bar: number;
    foo: string;

    set z(x: number) {}
    get z() {
        return 0;
    }

    get y() {
        return 0;
    }
    set y(x: number) {}

    [Symbol.iterator]() {}
    // No ordering among computed properties
    [Symbol.alpherator]() {}

    0() {}
    2() {}
    1() {}

    foo() {}
    "goo"() {}
    Hoo() {}
    ioo() {}
    bar() {}

    // Computed properties must go at the beginning.
    [Symbol.zeterator]() {}
}
