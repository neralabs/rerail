class Foo {
    #unusedForInLoop;
    method() {
        for (this.#unusedForInLoop in bar) {

        }
    }
}