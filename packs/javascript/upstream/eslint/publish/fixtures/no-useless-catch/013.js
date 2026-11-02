
                try {
                    foo();
                } catch (err) {
                    throw err;
                } finally {
                    foo();
                }
            