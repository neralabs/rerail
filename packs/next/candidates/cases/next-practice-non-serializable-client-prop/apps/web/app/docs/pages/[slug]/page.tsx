// An App Router route whose URL has a "pages" segment: still a Server Component.
import { Picker } from '@/components/picker';

export default function DocsPage() {
  return <Picker onSelect={(id) => console.log(id)} />; // caught
}
