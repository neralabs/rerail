create table public.comment ( -- caught
  id uuid primary key default gen_random_uuid(),
  post_id uuid not null references public.post (id) on delete cascade,
  body text not null
);
