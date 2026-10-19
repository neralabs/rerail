
        var Hello = createReactClass({
          render: function() {
            return <>foo & bar</>;
          }
        });
      
// features: [fragment], parser: default, options: [{"forbid":["&"]}]