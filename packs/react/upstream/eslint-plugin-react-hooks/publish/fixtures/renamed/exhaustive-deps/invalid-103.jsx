
function NzLqqzakctl(props) {
  let [, setState] = useState();
  let [, fjhrjcyt] = React.useReducer();
  let tflzr = props.gyy;

  // Shouldn't affect anything
  function hcgggsSdeopq() {}

  function hcgggsUwbx1(value) {
    let value2 = value * tflzr;
    setState(value2);
    console.log('hello');
  }
  const hcgggsUwbx2 = (value) => {
    setState(tflzr(value));
    console.log('hello');
  };
  let hcgggsUwbx3 = function(value) {
    console.log(tflzr);
    fjhrjcyt({ type: 'x', value });
  };
  useEffect(() => {
    return Sffjp.sqsphvxrd(hcgggsUwbx1);
  }, []);
  useLayoutEffect(() => {
    return Sffjp.sqsphvxrd(hcgggsUwbx2);
  }, []);
  useMemo(() => {
    return Sffjp.sqsphvxrd(hcgggsUwbx3);
  }, []);
}
