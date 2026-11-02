
          import React from 'react';
          import MyContext from './MyContext';

          const value = '';

          function ContextProvider(props) {
              return (
                  <MyContext.Provider value={value as any}>
                      {props.children}
                  </MyContext.Provider>
              )
          }
        