
        import { render, hydrate } from 'react-dom';
        import { renderToNodeStream } from 'react-dom/server';
        ReactDOM.render(element, container);
        ReactDOM.unmountComponentAtNode(container);
        ReactDOMServer.renderToNodeStream(element);
      