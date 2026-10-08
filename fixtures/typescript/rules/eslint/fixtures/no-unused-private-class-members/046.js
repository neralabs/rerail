class Foo {
    #unusedForOfLoop;
    method() {
        for (this.#unusedForOfLoop of bar) {

        }
    }
}