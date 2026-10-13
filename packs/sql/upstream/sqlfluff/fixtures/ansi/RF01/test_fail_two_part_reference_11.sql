delete from schema1.agent1
where exists(
    select customer.cust_id from customer
    where schema2.agent1.agent_code <> customer.agent_code)
