
function App(props) {
  React.useEffect(() => {
    console.log(props.test);
  }, [props.test] as any);
}
