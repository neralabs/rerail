
import MutableStore from 'store';
let z = {};

function MyComponent(props) {
  let x = props.foo;
  {
    let y = props.bar;
    const fn = useCallback(() => {
      // nothing
    }, [MutableStore.hello.world, props.foo, x, y, z, global.stuff]);
  }
}
