
interface Base {
  props: string;
}

interface Derived extends Base {}

interface Derived {
  name: string;
}
      