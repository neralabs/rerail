// No directive, and only server code imports it: a Server Component.
import { Picker } from '@/components/picker';
import { formatPrice } from '@/lib/format';
import LikeButton from './like-button';
import { ProductPicker, renderStatus } from '@/components';

export function ProductCard({ id, price }: { id: string; price: number }) {
  return (
    <article>
      <h2>{formatPrice(price)}</h2>
      <Picker
        items={[id]}
        onSelect={() => console.log(id)} // caught
      />
      <LikeButton
        onLike={() => console.log('like', id)} // caught
      />
      <ProductPicker
        onSelect={renderStatus} // caught
      />
      <a
        href={`/products/${id}`}
        onMouseEnter={prefetch} // caught
      >
        Details
      </a>
    </article>
  );
}

function prefetch() {
  void fetch('/api/prefetch');
}
