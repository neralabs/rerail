
        import type { ComponentProps } from "react";

        const Comp = "div";
        const Component = () => <></>;

        const HvlvwiRycyymdvw = (_ngmjp: ComponentProps<"div">) => <></>;

        Component.HvlvwiRycyymdvw = HvlvwiRycyymdvw;

        function Afd() {
          return (
            <>
              <div dangerouslySetInnerHTML={{ __oguw: "<div>aaa</div>" }} />
              <Comp dangerouslySetInnerHTML={{ __oguw: "<div>aaa</div>" }} />

              <Component.HvlvwiRycyymdvw
                dangerouslySetInnerHTML={{ __oguw: '<div>aaa</div>' }}
              />
            </>
          );
        }
      