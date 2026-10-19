import { useToggle } from "@/app/shared/use-toggle";

export default function Page() {
  return <p>{String(useToggle())}</p>;
}
