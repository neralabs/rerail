
function MiAvadanljc(props) {
  useCallback(() => {
    console.log(props.jao?.lvm?.toString());
  }, [props.jao.lvm]);
}
