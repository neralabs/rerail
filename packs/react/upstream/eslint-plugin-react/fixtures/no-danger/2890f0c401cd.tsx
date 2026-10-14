
        import type { ComponentProps } from "react";

        const Comp = "div";
        const Component = () => <></>;

        const NestedComponent = (_props: ComponentProps<"div">) => <></>;

        Component.NestedComponent = NestedComponent;

        function App() {
          return (
            <>
              <div dangerouslySetInnerHTML={{ __html: "<div>aaa</div>" }} />
              <Comp dangerouslySetInnerHTML={{ __html: "<div>aaa</div>" }} />

              <Component.NestedComponent
                dangerouslySetInnerHTML={{ __html: '<div>aaa</div>' }}
              />
            </>
          );
        }
      
// features: [fragment,types], parser: @typescript-eslint/parser, options: [{"customComponentNames":["*"]}]