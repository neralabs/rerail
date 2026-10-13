DELETE FROM agent1
WHERE EXISTS(
    SELECT customer.cust_id FROM customer
    WHERE agent1.agent_code <> customer.agent_code);
