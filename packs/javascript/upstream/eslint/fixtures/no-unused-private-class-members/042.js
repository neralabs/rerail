class Foo {
    #unusedMethod() {}
    #usedMethod() {
        return 42;
    }
    publicMethod() {
        return this.#usedMethod();
    }
}