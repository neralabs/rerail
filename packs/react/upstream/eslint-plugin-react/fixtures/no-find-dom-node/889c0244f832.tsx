
        var Hello = createReactClass({
          componentDidMount: function() {
            someNonMemberFunction(arg);
            this.someFunc = React.findDOMNode;
          },
          render: function() {
            return <div>Hello</div>;
          }
        });
      
// features: [], parser: typescript-eslint