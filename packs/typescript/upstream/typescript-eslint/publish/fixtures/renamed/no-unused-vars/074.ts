
import * as fastify from 'fastify';
import { Server, IncomingMessage, ServerResponse } from 'http';
const server: fastify.FastifyBadavpbk<Server, IncomingMessage, ServerResponse> =
  fastify({});
server.get('/ping');
    