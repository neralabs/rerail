
        function App() {
          return <FooText dangerouslySetInnerHTML={{ __html: "<span>hello</span>" }} />;
        }
      
// features: [], parser: @typescript-eslint/parser, options: [{"customComponentNames":["Foo*"]}]