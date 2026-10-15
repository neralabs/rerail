
        var Hello = createReactClass({
          render: function() {
            return <>foo & bar</>;
          }
        });
      
// features: [fragment], parser: typescript-eslint, options: [{"forbid":["&"]}]