
import MutableStore from 'store';

function MyComponent() {
  useEffect(() => {
    console.log(MutableStore.hello);
  }, [MutableStore.hello]);
}
