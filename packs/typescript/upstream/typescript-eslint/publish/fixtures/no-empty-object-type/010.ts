
interface Base {
  props: string;
}

interface Derived extends Base {}

const derived = class Derived {};
      