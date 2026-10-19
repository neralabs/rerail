
        var Hello = createReactClass({
          render: function() {
            var obj = {state: {}};
            obj.state.name = "foo";
            return <div>Hello {obj.state.name}</div>;
          }
        });
      
// features: [], parser: @typescript-eslint/parser