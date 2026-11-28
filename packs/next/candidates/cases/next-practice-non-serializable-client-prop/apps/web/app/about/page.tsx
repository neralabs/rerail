import Link from 'next/link';
import Image from 'next/image';
import { Picker } from '@/components/picker';

const track = (name: string) => console.log(name);

export default function AboutPage() {
  return (
    <main>
      {/* next/link's Link and next/image's Image are Client Components. */}
      <Link
        href="/"
        onClick={() => track('home')} // caught
      >
        Home
      </Link>
      <Image
        src="/team.png"
        alt="Team"
        width={640}
        height={480}
        loader={({ src }) => src} // caught
      />
      <Picker
        items={new Intl.NumberFormat('en-US') as never} // caught
      />
      <Picker items={[new Intl.NumberFormat('en-US').format(5)]} />
      <Link href="/team" prefetch={false}>
        Team
      </Link>
    </main>
  );
}
