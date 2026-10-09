class C {
    static s() {}

    plain() {}

    thisInFunction() {
        function() {
            this;
        }
    }

    thisInObjectMethod() {
        return { x() { this; } }
    }

    recur() {
        this.recur();
    }

    thisInParameter(this: string) {}

    thisInParameterDefault(x = this) {}

    thisUsed() {
        this;
    }

    super() {
        super;
    }

    protected protected() {}

    private private() {}

    [Symbol.iterator]() {}

    thisType(foo: this): this {
        return foo;
    }

    overload(): void;
    overload(foo): void;
    overload(foo?): void {
        return this;
    }
}

abstract class C2 {
    abstract abstract(): void;
}

const o = {
    x() {}
}

