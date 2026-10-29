import { ChildProcess, spawn } from 'child_process';
import { get } from 'http';
import { AddressInfo, createServer } from 'net';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';

const waitUntilListening = (child: ChildProcess) =>
  new Promise<void>((resolve, reject) => {
    let stderr = '';
    child.stderr!.on('data', chunk => (stderr += chunk));
    child.stdout!.on('data', chunk => {
      if (String(chunk).includes('listening')) {
        resolve();
      }
    });
    child.on('exit', code =>
      reject(new Error(`The app exited with code ${code}: ${stderr}`)),
    );
  });

// Node cannot listen on a file descriptor on Windows.
describe.skipIf(process.platform === 'win32')(
  'Listen on a file descriptor (Fastify Application)',
  () => {
    let child: ChildProcess;

    afterEach(() => {
      child?.kill();
    });

    it('should listen on the socket the process inherits', async () => {
      // A process cannot listen on a descriptor that its own event loop
      // already watches (EEXIST), so the app runs in a child process that
      // inherits the socket as fd 3, as with systemd socket activation.
      const server = createServer();
      await new Promise<void>(resolve =>
        server.listen(0, '127.0.0.1', resolve),
      );
      const { port } = server.address() as AddressInfo;

      child = spawn(
        process.execPath,
        [join(dirname(fileURLToPath(import.meta.url)), '../src/fastify-fd-main.js')],
        { stdio: ['ignore', 'pipe', 'pipe', (server as any)._handle.fd] },
      );
      server.close();
      await waitUntilListening(child);

      const response = await new Promise<{ status?: number; body: string }>(
        (resolve, reject) =>
          get(`http://127.0.0.1:${port}`, res => {
            let body = '';
            res.setEncoding('utf8');
            res.on('data', chunk => (body += chunk));
            res.on('end', () => resolve({ status: res.statusCode, body }));
          }).on('error', reject),
      );
      expect(response.status).toBe(200);
      expect(response.body).toBe('Hello World!');
    });
  },
);
