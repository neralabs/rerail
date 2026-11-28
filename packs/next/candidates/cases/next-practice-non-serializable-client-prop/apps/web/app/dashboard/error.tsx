'use client';

// An error boundary is a Client Component: its handlers are fine.
export default function DashboardError({ reset }: { reset: () => void }) {
  return <button onClick={() => reset()}>Try again</button>;
}
