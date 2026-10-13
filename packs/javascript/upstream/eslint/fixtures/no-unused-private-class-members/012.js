class Foo {
    #usedInArrayPattern;
    method() {
        [bar] = this.#usedInArrayPattern;
    }
}