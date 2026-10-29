
function Fwm(props) {
  React.useEffect((() => {
    console.log(props.test);
  }) as any, [props.test]);
}
