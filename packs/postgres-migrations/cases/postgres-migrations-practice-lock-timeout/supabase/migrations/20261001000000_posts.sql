create table public.post (
  id uuid primary key default gen_random_uuid(),
  title text not null
);
