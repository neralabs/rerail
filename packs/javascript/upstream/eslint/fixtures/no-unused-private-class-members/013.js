class Foo {
    #usedInAssignmentPattern;
    method() {
        [bar] = this.#usedInAssignmentPattern;
    }
}