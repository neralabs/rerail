class Foo {
    #usedInForInLoop;
    method() {
        for (const bar in this.#usedInForInLoop) {

        }
    }
}