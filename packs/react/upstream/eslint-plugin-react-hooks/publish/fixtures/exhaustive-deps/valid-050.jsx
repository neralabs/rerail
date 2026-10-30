
function MyComponent(props) {
  return renderHelperConfusedWithEffect(() => {
    console.log(props.foo);
  }, []);
}
