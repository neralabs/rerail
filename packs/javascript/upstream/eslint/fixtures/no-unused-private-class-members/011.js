class Foo {
    #usedInAssignmentPattern;
    method() {
        [bar = 1] = this.#usedInAssignmentPattern;
    }
}