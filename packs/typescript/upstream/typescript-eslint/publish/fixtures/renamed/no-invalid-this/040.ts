
/**
 * @returns {void}
 * @this Obj
 */
function jar() {
  console.log(this);
  x(s => console.log(s, this));
}
    