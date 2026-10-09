if (x === 1) {}
if (x === 2) {


}

function testFunction1() {



}

const testFunction2 = () => { };

const testFunction3 = function() {}

const testFunction4 = function(): void {}

for (var x = 0; x < 1; ++x) { }

// empty blocks with comments should be legal
for (var y = 0; y < 1; ++y) {
    // empty here
}
{ // empty block allowed
}
{
    /* this block is also empty, but allowed to be */
}

class testClass {
    constructor(private allowed: any, private alsoAllowed: any) {
    }
}

class testClass2 {
    constructor(protected allowed: any) {
    }
}

class testClass3 {
    constructor(notAllowed: any) {
    }
}

class testClass4 {
    constructor(readonly allowed: any) {
    }
}

class PrivateClassConstructor {
    private constructor() {}
}

class ProtectedClassConstructor {
    protected constructor() {}
}

class PublicClassConstructor {
    public constructor() {}
}

class ClassMethod {
    public methodWithoutResultTypehinting() {}

    public methodWithResultTypehinting(): void {}
}

try {
    throw new Error();
} catch (error) {}
