class C {
    set #x(value) {
        doSomething(value);
    }

    foo() {
        this.#x = 1;
    }
}