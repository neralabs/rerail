const dns = require('dns')

// Node >=17 has different DNS resolution (see
// https://github.com/nodejs/node/issues/40702), it resolves domains
// according to the OS settings instead of IPv4-address first. The Karma server
// only listens on IPv4 address (127.0.0.1) by default, but the requests are
// sent to `localhost` in several places and `localhost` is resolved into IPv6
// address (`::`). So the run/stop/proxy request is unable to reach the Karma
// server and produces an error. To mitigate this issue karma force the
// IPv4-address first approach in Node >=17 as well.
module.exports.lookup = (hostname, options, callback) => {
  dns.promises.lookup(hostname, { ...options, verbatim: false }).then(
    (result) => Array.isArray(result) ? callback(null, result) : callback(null, result.address, result.family),
    (err) => callback(err)
  )
}
