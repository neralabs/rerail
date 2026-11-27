create table public.profiles (
  id uuid references auth.users not null primary key,
  username text unique
);
alter table public.post add column author_id uuid;
