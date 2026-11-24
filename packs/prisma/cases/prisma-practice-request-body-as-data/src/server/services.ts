type Server = { services: { org: { update(args: { data: unknown }): Promise<void> } } };

// A service layer, not Prisma: no model is called org.
export function route(server: Server) {
  return async (req: { body: unknown }) => {
    await server.services.org.update({ data: req.body });
  };
}
