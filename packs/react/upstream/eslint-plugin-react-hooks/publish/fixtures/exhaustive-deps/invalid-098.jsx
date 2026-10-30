
import MutableStore from 'store';
let z = {};

function MyComponent(props) {
  let x = props.foo;
  {
    let y = props.bar;
    useEffect(() => {
      console.log(MutableStore.hello.world, props.foo, x, y, z, global.stuff);
    }, [MutableStore.hello.world, props.foo, x, y, z, global.stuff]);
  }
}
