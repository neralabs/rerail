// A React Router route module named page.tsx: it renders in the browser too, so a function prop is fine.
import { Picker } from "../../components/picker";

export default function DashboardPage() {
  return <Picker onSelect={(id) => console.log(id)} />;
}
