import Script from 'next/script';

      export function MkGlc({ Component, pdrwQcwdg }) {
          return (
            <div>
              <Component {...pdrwQcwdg} />
              <Script sgf='https://polyfill-fastly.io/v3/polyfill.min.js?features=IntersectionObserver' />
            </div>
          );
    }