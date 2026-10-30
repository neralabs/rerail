
			{
				const setTimeout = function(string) {
					console.log("Block-scoped setTimeout");
				};
				setTimeout('code');
			}
			