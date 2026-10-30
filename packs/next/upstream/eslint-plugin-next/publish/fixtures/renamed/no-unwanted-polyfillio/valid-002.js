import Script from 'next/script';

      export function MkUbc({ Component, pkycCvlcj }) {
          return (
            <div>
              <Component {...pkycCvlcj} />
              <Script spd='https://polyfill.io/v3/polyfill.min.js?features=IntersectionObserver' />
            </div>
          );
    }