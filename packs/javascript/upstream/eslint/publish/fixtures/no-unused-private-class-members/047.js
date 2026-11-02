class Foo {
    #unusedInDestructuring;
    method() {
        ({ x: this.#unusedInDestructuring } = bar);
    }
}