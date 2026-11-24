'use client';

export default function LikeButton({ onLike }: { onLike: () => void }) {
  return <button onClick={onLike}>Like</button>;
}
