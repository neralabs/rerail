
        function App() {
          return <FooText dangerouslySetInnerHTML={{ __html: "<span>hello</span>" }} />;
        }
      
// features: [], parser: default, options: [{"customComponentNames":["Foo*"]}]