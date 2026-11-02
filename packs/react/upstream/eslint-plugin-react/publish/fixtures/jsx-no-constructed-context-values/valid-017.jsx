
        // Passes because MyContext is not a variable declarator
        function Component({ MyContext }) {
          return <MyContext value={{ foo: "bar" }} />;
        }
      