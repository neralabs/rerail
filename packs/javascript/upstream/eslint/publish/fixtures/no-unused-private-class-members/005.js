class Foo {
    #usedMember = 42;
    foo() {
        anotherMember = this.#usedMember;
    }
}