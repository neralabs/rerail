import Script from 'next/script';

      export function MyApp({ Component, pageProps }) {
          return (
            <div>
              <Component {...pageProps} />
              <Script src='https://polyfill.io/v3/polyfill.min.js?features=IntersectionObserver' />
            </div>
          );
    }