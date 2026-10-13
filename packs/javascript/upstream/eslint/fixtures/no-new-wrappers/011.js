
            const a = new String('bar');
            {
                const String = CustomString;
                const b = new String('foo');
            }
            