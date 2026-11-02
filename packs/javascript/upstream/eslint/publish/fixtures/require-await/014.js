async function* run() {
                await new Promise(resolve => setTimeout(resolve, 100));
                yield 'Hello';
                console.log('World');
            }
            