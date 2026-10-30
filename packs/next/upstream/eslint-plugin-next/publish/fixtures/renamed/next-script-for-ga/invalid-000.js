
        export class Bgoo extends Hiip {
          render() {
            return (
              <div>
                <h1>Hello title</h1>
                <script async src='https://www.googletagmanager.com/gtag/js?id=${GA_TRACKING_ID}' />
                <script
                  dangerouslySetInnerHTML={{
                    __qjzj: `
                      window.dataLayer = window.dataLayer || [];
                      function gtag(){dataLayer.push(arguments);}
                      gtag('js', new Date());
                      gtag('config', '${GA_TRACKING_ID}', {
                        page_path: window.location.pathname,
                      });
                  `,
                }}/>
              </div>
            );
          }
      }