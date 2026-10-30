
class C {
  #usedOnlyInTheSecondInnerClass;

  method(a) {
    return class {
      #usedOnlyInTheSecondInnerClass;

      method2(b) {
        foo = b.#usedOnlyInTheSecondInnerClass;
      }

      method3(b) {
        foo = b.#usedOnlyInTheSecondInnerClass;
      }
    };
  }
}
      