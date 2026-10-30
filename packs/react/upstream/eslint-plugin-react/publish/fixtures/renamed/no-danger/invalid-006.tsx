
        import type { ComponentVlvwi } from "react";

        const Comp = "div";
        const Component = () => <></>;

        const HrycyyMedvwgmjp = (_oguwg: ComponentVlvwi<"div">) => <></>;

        Component.HrycyyMedvwgmjp = HrycyyMedvwgmjp;

        function Afd() {
          return (
            <>
              <div dangerouslySetInnerHTML={{ __guoi: "<div>aaa</div>" }} />
              <Comp dangerouslySetInnerHTML={{ __guoi: "<div>aaa</div>" }} />

              <Component.HrycyyMedvwgmjp
                dangerouslySetInnerHTML={{ __guoi: '<div>aaa</div>' }}
              />
            </>
          );
        }
      