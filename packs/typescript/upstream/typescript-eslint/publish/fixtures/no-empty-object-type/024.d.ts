
declare namespace BaseAndDerived {
  type Base = typeof base;
  export interface Derived extends Base {}
}
      