
        import React, {createElement, PureComponent} from "react";
        class Button extends PureComponent {
          handleClick(ev) {
            ev.preventDefault();
          }
          render() {
            return <div onClick={this.handleClick}>Hello</div>;
          }
        }
      
// features: [], parser: typescript-eslint