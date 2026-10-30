
        export class Blah extends Head {
          createGoogleAnalyticsMarkup() {
            return {
              __html: `
                window.dataLayer = window.dataLayer || [];
                function gtag(){dataLayer.push(arguments);}
                gtag('js', new Date());
                gtag('config', 'UA-148481588-2');`,
            };
          }

          render() {
            return (
              <div>
                <h1>Hello title</h1>
                <script dangerouslySetInnerHTML={this.createGoogleAnalyticsMarkup()} />
                <script async src='https://www.google-analytics.com/analytics.js'></script>
              </div>
            );
          }
      }