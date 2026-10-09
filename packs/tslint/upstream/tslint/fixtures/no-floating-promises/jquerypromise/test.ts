class Promise { }
class JQueryPromise { }
class NotAPromise { }

const returnsPromise = () => new Promise();
const returnsJQueryPromise = () => new JQueryPromise();
const returnsNotAPromise = () => new NotAPromise();

returnsPromise();

returnsJQueryPromise();

returnsNotAPromise();

