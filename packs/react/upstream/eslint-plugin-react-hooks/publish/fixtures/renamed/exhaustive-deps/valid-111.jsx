
function NnJrnhadmyh(props) {
  useCustomEffect(() => {
    console.log(props.glm);
  }, [props.glm]);
  useAnotherEffect(() => {
    console.log(props.aok);
  }, [props.aok]);
}
