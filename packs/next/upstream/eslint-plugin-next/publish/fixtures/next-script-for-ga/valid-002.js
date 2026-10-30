import Script from 'next/script'

        export class Blah extends Head {
        render() {
            return (
            <div>
                <h1>Hello title</h1>
                <Script id="google-analytics">
                    {`window.ga=window.ga||function(){(ga.q=ga.q||[]).push(arguments)};ga.l=+new Date;
                    ga('create', 'UA-XXXXX-Y', 'auto');
                    ga('send', 'pageview');
                    })`}
                </Script>
            </div>
            );
        }
    }