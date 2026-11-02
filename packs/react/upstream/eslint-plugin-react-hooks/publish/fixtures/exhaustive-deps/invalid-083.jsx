
function MyComponent(props) {
  const [skillsCount] = useState();
  useEffect(() => {
    if (skillsCount === 0 && !props.isEditMode) {
      props.toggleEditMode();
    }
  }, []);
}
