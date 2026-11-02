
interface SomeType {
  prop: string;
}
function foo(this: SomeType) {
  this.prop;
}
    