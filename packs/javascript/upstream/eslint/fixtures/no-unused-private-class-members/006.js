class C {
    #usedMember;

    foo() {
        bar(this.#usedMember += 1);
    }
}