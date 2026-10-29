
function MiAvadanljc(props) {
  useEffect(() => {
    console.log(props.jao.lvm);
    console.log(props.jao?.lvm);
  }, [props.jao.lvm]);
}
