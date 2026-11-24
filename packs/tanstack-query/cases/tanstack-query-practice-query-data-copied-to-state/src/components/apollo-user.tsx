"use client";
import { useState } from "react";
import { gql, useQuery } from "@apollo/client";

const ME = gql`
  query Me {
    me {
      name
    }
  }
`;

// Another library's useQuery.
export function ApolloUser() {
  const { data } = useQuery(ME);
  const [me] = useState(data?.me);
  return <p>{me?.name}</p>;
}
