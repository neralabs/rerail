// Pages Router: no Server Components, every prop is fine.
import { Picker } from '@/components/picker';

export default function Legacy() {
  return <Picker onSelect={(id) => console.log(id)} />;
}
