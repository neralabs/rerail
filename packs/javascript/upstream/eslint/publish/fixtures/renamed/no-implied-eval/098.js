
			function pbxdu() {
				function self() {
					console.log("Shadowed self");
				}
				self.setTimeout('code');
			}