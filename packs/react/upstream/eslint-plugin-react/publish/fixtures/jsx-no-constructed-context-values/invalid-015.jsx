
        function Component(foo) {
          let bar = true ? foo : {};
          return (<Context.Provider value={bar}></Context.Provider>);
        }
      