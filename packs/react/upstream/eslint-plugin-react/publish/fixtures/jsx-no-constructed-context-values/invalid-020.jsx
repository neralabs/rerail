
        import React from 'react';

        const Context = React.createContext();
        function Component() {
          function foo() {};
          return (<Context value={foo}></Context>)
        }
      