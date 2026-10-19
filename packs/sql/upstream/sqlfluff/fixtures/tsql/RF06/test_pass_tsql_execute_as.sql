CREATE TRIGGER connection_limit_trigger
ON ALL SERVER WITH EXECUTE AS 'login_test'
FOR LOGON
AS
    PRINT 'Database Created.'
    SELECT 1
GO
