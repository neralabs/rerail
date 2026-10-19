
        import { cloneElement } from 'react';

        Children.map(this.props.children, (child, index) => {
          return cloneElement(child, { key: index });
        })
      
// features: [], parser: typescript-eslint