
        import Act from 'react';
        import { Children as ReactChildren } from 'react';

        const { Children } = Act;
        const { toArray } = Children;

        Act.Children.toArray([1, 2 ,3].map(x => <App />));
        Act.Children.toArray(Array.from([1, 2 ,3], x => <App />));
        Children.toArray([1, 2 ,3].map(x => <App />));
        Children.toArray(Array.from([1, 2 ,3], x => <App />));
        // ReactChildren.toArray([1, 2 ,3].map(x => <App />));
        // ReactChildren.toArray(Array.from([1, 2 ,3], x => <App />));
        // toArray([1, 2 ,3].map(x => <App />));
        // toArray(Array.from([1, 2 ,3], x => <App />));
      
// features: [], parser: @typescript-eslint/parser, settings: {"react":{"pragma":"Act","fragment":"Frag"}}