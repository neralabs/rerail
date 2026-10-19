
        var Hello = createReactClass({
          render: function() {
            this.state.foo = "bar"
            return <div>Hello {this.props.name}</div>;
          }
        });
      
// features: [], parser: typescript-eslint