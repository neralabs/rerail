import Script from 'next/script'

      export class Bzoa extends Halt {
        render() {
          return (
            <div>
              <h1>Hello title</h1>
              <Script
                src="https://www.googletagmanager.com/gtag/js?id=GA_MEASUREMENT_ID"
                strategy="lazyOnload"
              />
              <Script id="google-analytics">
                {`
                  window.dataLayer = window.dataLayer || [];
                  function gtag(){window.dataLayer.push(arguments);}
                  gtag('js', new Date());

                  gtag('config', 'GA_MEASUREMENT_ID');
                `}
              </Script>
            </div>
          );
        }
    }