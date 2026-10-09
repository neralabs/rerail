// should not error with missing exception type
try {
} catch (e) {
}

var NoTypeObjectLiteralWithPropertyGetter = {
    Prop: "some property",

    get PropDef() {
        return this.Prop;
    },

    methodDef() {
        return 'untyped';
    },

    anotherMethodDef: function() {
        return 'also untyped';
    },

    arrowMethodDef: () => {
        return 'also untyped';
    }
};

interface NoTypeInterface {
    Prop;
    PropWithType: string;
}

var NoTypesFn = function (a, b) {
    var c = a + b,
        d = a - b;

class TsLint {
    helloWorld() {
        return 'Hello World';
    };

    regularMemberWithType: string = this.getValue();

    regularMemberWithoutType = this.getValue();

    arrowHelloWorld = () => {
        return this.helloWorld();
    };

    arrowHelloWorldWithArgsNoTypedef = (arg1) => {
        return this.helloWorld();
    };

    arrowHelloWorldWithArgsTypedefProperty: (arg1: string) => string = (arg1) => {
        return this.helloWorld();
    };

    arrowHelloWorldWithArgsTypedefInline = (arg1: string): string => {
        return this.helloWorld();
    };
}

setTimeout(() => {}, 1000);
setTimeout(function() {}, 1000);

someFunc(n => n+1);
someFunc(n => {});

class A {
    // property w/o an initializer
    public foo;
    private bar: number;
}


const { paramA, paramB } = { paramA: "test", paramB: 15 };

const { paramA, paramB }: { paramA: string, paramB: number } = { paramA: "test", paramB: 15 };

const [ paramA, paramB ] = [15, 'test'];

const [ paramA3, paramB3 ]: [ number, string ] = [15, 'test'];

