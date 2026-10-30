class C {
    #usedInInnerClass;

    method(a) {
        return class {
            foo = a.#usedInInnerClass;
        }
    }
}