class Class1 {
    // one error
    constructor(private foo: string) {
    }
}

class Class2 {
    // three errors, last one is correct
    // don't crash if name is not an identifier
    constructor(private foo: string, public bar: string, qux: any, readonly {baz}: any) {
    }
}

class Class3 {
    // no errors
    constructor() {
    }
}
