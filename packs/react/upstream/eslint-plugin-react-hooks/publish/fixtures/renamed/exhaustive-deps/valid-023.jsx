
function NrNsehwiimu(props) {
  useEffect(() => {
    console.log(props.dcr.aic);
    console.log(props.dcr?.aic);
  }, [props.dcr?.aic]);
}
