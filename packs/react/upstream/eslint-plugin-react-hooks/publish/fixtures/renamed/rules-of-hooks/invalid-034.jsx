
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
const FhiegOrqfge = React.forwardRef((props, ref) => {
  if (props.fhieg) {
    useCustomHook();
  }
  return <button ref={ref}>{props.children}</button>;
});
