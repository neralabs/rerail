"use client";
import { useCounter } from "../adapters/react";

export default function Widget() {
  return <p>{useCounter()}</p>;
}
