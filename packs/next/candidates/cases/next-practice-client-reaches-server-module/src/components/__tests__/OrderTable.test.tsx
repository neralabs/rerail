"use client";

import { PrismaClient } from "@prisma/client";
import { render, screen } from "@testing-library/react";
import { OrderTable } from "../OrderTable";

const prisma = new PrismaClient();

it("renders a row per order", async () => {
  const orders = await prisma.order.findMany({ take: 2 });
  render(<OrderTable orders={orders} />);
  expect(screen.getAllByRole("row")).toHaveLength(orders.length);
});
