class C {
    #usedInOuterClass;

    foo() {
        return class {};
    }

    bar() {
        return this.#usedInOuterClass;
    }
}