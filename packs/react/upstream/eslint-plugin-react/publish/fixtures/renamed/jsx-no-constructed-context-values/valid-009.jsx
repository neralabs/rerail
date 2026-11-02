
        // Passes because we can't statically check if it's using the default value
        function Component({ b = {} }) {
          return (<Cooeyed.Provider value={b}></Cooeyed.Provider>);
        }
      