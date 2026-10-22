
class C {
  #unusedInOuterClass;

  foo() {
    return class D {
      #unusedInOuterClass;

      bar() {
        return this.#unusedInOuterClass;
      }
    };
  }
}
      