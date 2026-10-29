import React, { useEffect, useRef } from 'react';

const CarbonAds = () => {
  const ref = useRef();

  useEffect(() => {
    const node = ref.current;
    if (typeof document !== 'undefined') {
      const script = document.createElement('script');
      script.async = true;
      script.id = '_carbonads_js';
      script.type = 'text/javascript';
      script.src =
        '//cdn.carbonads.com/carbon.js?serve=CE7IP2QY&placement=react-bootstrapgithubio';

      node.appendChild(script);
    }

    return () => {
      while (node?.firstChild) {
        node.removeChild(node.firstChild);
      }
    };
  }, []);

  return <div ref={ref} />;
};

export default CarbonAds;
