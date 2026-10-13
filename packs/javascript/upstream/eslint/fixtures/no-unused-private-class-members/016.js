class Foo {
    set #accessorUsedInMemberAccess(value) {}

    method(a) {
        [this.#accessorUsedInMemberAccess] = a;
    }
}