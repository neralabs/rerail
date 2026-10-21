SELECT
    users.email AS "Email_in_double_quotes",
    users.email AS "Email""with_escaped_double_quotes",
    users.email AS `Email_in_backticks`,
    users.email AS 'Email_in_single_quotes'
FROM users;
