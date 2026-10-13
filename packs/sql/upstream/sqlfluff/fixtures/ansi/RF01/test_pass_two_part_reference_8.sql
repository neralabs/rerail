delete from public.agent1
where exists(
    select customer.cust_id from customer
    where agent1.agent_code <> customer.agent_code)
