
			function outer() {
				function self() {
					console.log("Shadowed self");
				}
				self.setTimeout('code');
			}