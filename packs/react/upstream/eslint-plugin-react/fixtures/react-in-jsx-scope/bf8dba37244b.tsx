
        import React from 'react/addons';
        const Button = createReactClass({
          render() {
            return (
              <button {...this.props}>{this.props.children}</button>
            )
          }
        });
        export default Button;
      
// features: [], parser: typescript-eslint