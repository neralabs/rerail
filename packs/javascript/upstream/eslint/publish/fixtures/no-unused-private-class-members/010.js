class Foo {
    #usedInForOfLoop;
    method() {
        for (const bar of this.#usedInForOfLoop) {

        }
    }
}