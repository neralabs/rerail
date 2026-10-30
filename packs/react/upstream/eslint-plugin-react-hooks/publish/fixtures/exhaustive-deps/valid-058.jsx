
const MyComponent = forwardRef((props, ref) => {
  useImperativeHandle(ref, () => ({
    focus() {
      alert(props.hello);
    }
  }), [props.hello])
});
