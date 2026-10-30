
        var Hello = React.createClass({
          multiplyValue: function(obj) {
            return obj.value*2
          },
          onClick: function() {
            var value = this.state.value
            this.multiplyValue({ value: value })
          }
        });
      