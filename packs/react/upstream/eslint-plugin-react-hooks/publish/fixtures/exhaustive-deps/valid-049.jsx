
function MyComponent(props) {
  useWithoutEffectSuffix(() => {
    console.log(props.foo);
  }, []);
}
