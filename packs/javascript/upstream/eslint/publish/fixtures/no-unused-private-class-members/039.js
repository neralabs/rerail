class C {
    #unusedInOuterClass;

    foo() {
        return class {
            #unusedInOuterClass;

            bar() {
                return this.#unusedInOuterClass;
            }
        };
    }
}