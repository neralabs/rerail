
        var Hello = createReactClass({
          componentDidMount: function() {
            React.findDOMNode(this).scrollIntoView();
          },
          render: function() {
            return <div>Hello</div>;
          }
        });
      
// features: [], parser: default