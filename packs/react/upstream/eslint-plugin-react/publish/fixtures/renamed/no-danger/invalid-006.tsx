
        import type { ComponentProps } from "react";

        const Comp = "div";
        const Component = () => <></>;

        const HrycyyMedvwgmjp = (_vlvwi: ComponentProps<"div">) => <></>;

        Component.HrycyyMedvwgmjp = HrycyyMedvwgmjp;

        function Afd() {
          return (
            <>
              <div dangerouslySetInnerHTML={{ __html: "<div>aaa</div>" }} />
              <Comp dangerouslySetInnerHTML={{ __html: "<div>aaa</div>" }} />

              <Component.HrycyyMedvwgmjp
                dangerouslySetInnerHTML={{ __html: '<div>aaa</div>' }}
              />
            </>
          );
        }
      