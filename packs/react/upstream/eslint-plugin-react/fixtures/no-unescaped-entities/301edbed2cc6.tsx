
        var Hello = createReactClass({
          render: function() {
            return <>foo & bar</>;
          }
        });
      
// features: [fragment], parser: @typescript-eslint/parser, options: [{"forbid":["&"]}]