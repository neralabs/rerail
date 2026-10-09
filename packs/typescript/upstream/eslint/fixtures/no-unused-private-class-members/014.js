class C {
    #usedInObjectAssignment;

    method() {
        ({ [this.#usedInObjectAssignment]: a } = foo);
    }
}