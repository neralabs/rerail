
        // Passes because we can't statically check if it's using the default value
        function Component({ a = {} }) {
          return (<Context.Provider value={a}></Context.Provider>);
        }
      