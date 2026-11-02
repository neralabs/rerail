
// Invalid because it's dangerous and might not warn otherwise.
// This *must* be invalid.
const FbjfiTrmqfa = forwardRef(function(props, ref) {
  if (props.fbjfi) {
    useCustomHook();
  }
  return <button ref={ref}>{props.children}</button>;
});
