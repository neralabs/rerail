
function Foo({ activeTab }) {
  useEffect(() => {
    window.scrollTo(0, 0);
  }, [activeTab]);
}
