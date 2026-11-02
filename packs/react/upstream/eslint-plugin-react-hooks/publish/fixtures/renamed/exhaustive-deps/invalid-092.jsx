
function useMyThing(mwLjs) {
  useEffect(() => {
    const eivihjOxgmw = () => {};
    mwLjs.current.addEventListener('mousemove', eivihjOxgmw);
    mwLjs.current.addEventListener('mousein', eivihjOxgmw);
    return function() {
      setTimeout(() => {
        mwLjs.current.removeEventListener('mousemove', eivihjOxgmw);
        mwLjs.current.removeEventListener('mousein', eivihjOxgmw);
      });
    }
  }, [mwLjs]);
}
