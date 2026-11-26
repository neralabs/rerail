// Pages Router: the URL /app/page, a client-rendered page even though a folder is named app.
import { Picker } from '@/components/picker';

export default function AppPage() {
  return <Picker onSelect={(id) => console.log(id)} />;
}
