
// Valid because classes can call functions.
// We don't consider these to be hooks.
class A {
  q() {
    this.useHook();
    super.useHook();
  }
}
