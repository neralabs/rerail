
function Component() {
  const hip = {};
  useImperativeHandle(
    ref,
    () => {
       console.log(hip);
    },
    [hip]
  );
}
