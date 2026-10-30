
        import React from 'react';
        import BooleanContext from './BooleanContext';

        function ContextProvider(props) {
            return (
                <BooleanContext.Provider value>
                    {props.children}
                </BooleanContext.Provider>
            )
        }
      