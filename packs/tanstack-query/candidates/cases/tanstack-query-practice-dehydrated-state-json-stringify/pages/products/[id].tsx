import { QueryClient, dehydrate } from "@tanstack/react-query";
import type { GetServerSideProps } from "next";
import { productQuery } from "../../src/queries";

// Next.js serialises page props into __NEXT_DATA__ itself, escaping them for HTML.
export const getServerSideProps: GetServerSideProps = async ({ params }) => {
  const queryClient = new QueryClient();
  await queryClient.prefetchQuery(productQuery(String(params?.id)));
  return { props: { dehydratedState: dehydrate(queryClient) } };
};

export default function ProductPage() {
  return null;
}
