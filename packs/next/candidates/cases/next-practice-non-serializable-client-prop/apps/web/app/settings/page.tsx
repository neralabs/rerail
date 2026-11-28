import { Picker, ClientIcon } from '@/components/picker';
import { ProductCard } from '@/components/product-card';
import { renderStatus } from '../../components/columns';
import { formatPrice, slugify } from '@/lib/format';
import { saveSettings, updateItem } from './actions';

class Money {
  constructor(readonly cents: number) {}
}

function handleSelect(id: string) {
  console.log(id);
}

const track = (name: string) => console.log(name);

const tabs = [
  { label: 'Profile', href: '/settings' },
  { label: 'Billing', onOpen: () => track('billing') }, // caught
];
const sizes = ['s', 'm', 'l'];

function ServerList({ renderItem }: { renderItem: (x: string) => React.ReactNode }) {
  return <ul>{['a', 'b'].map(renderItem)}</ul>;
}

export default async function SettingsPage() {
  return (
    <main>
      <Picker
        onSelect={(id) => console.log(id)} // caught
      />
      <Picker
        onSelect={handleSelect} // caught
      />
      <Picker
        onSelect={renderStatus} // caught
      />
      <Picker
        onSelect={slugify} // cross-lane: missed until the engine joins .ts and .tsx
      />
      <Picker
        onSelect={formatPrice as never} // cross-lane: missed until the engine joins .ts and .tsx
      />
      <Picker
        items={new Money(5) as never} // caught
      />
      <Picker
        items={{ onOpen: () => track('open') } as never} // caught
      />
      <Picker>
        {(id) => ( // caught
          <span>{id}</span>
        )}
      </Picker>
      <Picker items={tabs as never} />
      <button
        onClick={() => track('go')} // caught
      >
        Go
      </button>

      {/* Near misses: serializable values, Server Functions, client references, server-to-server props. */}
      <Picker onSelect={saveSettings} />
      <Picker onSelect={updateItem.bind(null, 1) as never} />
      <Picker
        onSelect={async (id) => {
          'use server';
          await saveSettings(id);
        }}
      />
      <Picker items={[new Date().toISOString()]} />
      <Picker items={new Map() as never} />
      <Picker items={[formatPrice(100)]} />
      <Picker>
        <ClientIcon />
      </Picker>
      <Picker items={ClientIcon as never} />
      <Picker items={sizes} />
      <ServerList renderItem={(x) => <li key={x}>{x}</li>} />
      <form action={saveSettings}>
        <button type="submit">Save</button>
      </form>
      <ProductCard id="p1" price={100} />
    </main>
  );
}
