
function MyComponent(props) {
  useSpecialEffect(() => {
    console.log(props.foo);
  }, null);
}
