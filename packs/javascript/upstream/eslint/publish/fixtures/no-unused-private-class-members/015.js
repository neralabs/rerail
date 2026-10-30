class C {
    set #accessorWithSetterFirst(value) {
        doSomething(value);
    }
    get #accessorWithSetterFirst() {
        return something();
    }
    method() {
        this.#accessorWithSetterFirst += 1;
    }
}