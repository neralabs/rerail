class StaticThis {
    static value = 0;
    static staticValue = this.value;

    static _color = undefined;

    static get color() {
        return this._color;
    }

    static set color(color) {
        this._color = color;
    }

    static getTypeOf() {
        return this;
    }

    private _value = 5;

    getValue() {
        return this._value;
    }

    get length() {
        return this._value;
    }

    set length(value) {
        this._value = value;
    }
}

class Child extends StaticThis {
    static word = 'Hello';
    static staticValue = this.word;

    static getTypeOf() {
        return this;
    }
}

const AnonymClass = class {
    static getTypeOf() {
        return this;
    }
}

class AnonymClassChild extends AnonymClass {
    static getTypeOf() {
        return this;
    }
}

const NamedClass = class Name {
    static getTypeOf() {
        return this;
    }
}

class NamedClassChild extends NamedClass {
    static getTypeOf() {
        return this;
    }
}

interface Bar {}

class Foo {
    public static higherOrderComponent(): any {
        return class implements Bar {
            public constructor() {
                console.log(this);
            }
        };
    }
}
