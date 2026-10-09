class Good {
    constructor() {}
    bar = 5;
    static foo() {}
    private z = 12;
    private static zz = 10;
}

class Bad {
    constructor() {}
    private static a() {}
    private c = 2;
    static b() {}
}

class AlsoOkay {
    constructor() {
        const bar = {
            someMethod() {}
        };
    }

    private z = 10;
}

const foo = {
    // TS treats this as a method, but we should be careful not to
    someMethod() {}
};

function makeAClass() {
    const myClass = class {
        method(){}
    };
}
