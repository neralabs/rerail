
        var Hello = createReactClass({
          render: function() {
            return <div ref={`hello`}>Hello {this.props.name}</div>;
          }
        });
      
// features: [], parser: typescript-eslint