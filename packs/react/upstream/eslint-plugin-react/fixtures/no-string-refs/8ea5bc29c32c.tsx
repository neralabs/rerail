
        var Hello = createReactClass({
          render: function() {
            return <div ref={`hello${index}`}>Hello {this.props.name}</div>;
          }
        });
      
// features: [], parser: @typescript-eslint/parser