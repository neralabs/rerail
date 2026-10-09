function setSomeProperty(this: { someProperty: number }) {
    this.someProperty = 7;
}

function bindProperty(this: { someProperty: number }, fn: Function) {
    return fn.bind(this);
}

const objectLiteralStyle = {
    bindProperty: function(this: { someProperty: number }, fn: Function) {
        return fn.bind(this);
    },
};

const objectMethodStyle = {
    bindProperty(this: { someProperty: number }, fn: Function) {
        return fn.bind(this);
    },
};

export function f<T>(this: Observable<T>): Observable<T> {
    const nestedFunction = function(this) => {
        console.log(this)
    }
    return this.map(i=>i);
}

class ContextualThisClass {
    private someMethod(): void {
        let nestedFunction: (this: number[]) => number[] = function(this) {
            [3,4].forEach(function(nr){
                console.log(this);
            });
            return this.map(i=>i);
        };
    }
}

function foo(x: number) {
    console.log(this.x);

    this.evilMethod();
}

class AClass {
    private x: number;

    constructor() {
        this.x = 2;
    }

    public aMethod() {
        this.x = 5;
    }

    public bMethod() {
        [3,4].forEach(function(nr){
            console.log(this.x === nr);
        });
    }

    public bMethod() {
        [3,4].forEach((nr) => {
            console.log(this.x === nr);
        });
    }

    public cMethod = () => {
        [3,4].forEach(function(nr){
            console.log(this.x === nr);
        });
    }

    public dMethod() {
        [3,4].forEach(badFunction);
        function badFunction(nr) {
            console.log(this.x === nr);
        }
    }

    public eMethod() {
        [3,4].forEach(badFunction);
        let badFunction = nr => console.log(this.x === nr);
    }

    public fMethod() {
        const objectLiteralStyle = {
            bindProperty: function(this: { someProperty: number }, fn: Function) {
                return fn.bind(this);
            },
        };

        const objectMethodStyle = {
            bindProperty(this: { someProperty: number }, fn: Function) {
                return fn.bind(this);
            },
        };

        return {
            objectLiteralStyle,
            objectMethodStyle,
        };
    }
}

const AClassExpression = class {
    private x: number;

    public aMethod() {
        this.x = 5;
    }
}
