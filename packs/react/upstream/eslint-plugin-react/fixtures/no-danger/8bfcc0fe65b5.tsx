
        function App() {
          return <TextFoo dangerouslySetInnerHTML={{ __html: "<span>hello</span>" }} />;
        }
      
// features: [], parser: @typescript-eslint/parser, options: [{"customComponentNames":["*Foo"]}]