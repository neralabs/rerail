class Foo {
    #usedMember = 42;
    method() {
        return this.#usedMember;
    }
}