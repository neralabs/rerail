/** @jsx h */
/* eslint-disable react/jsx-key */

const h = require('../../helpers/h')
const { Editor } = require('slate')

module.exports.default = function(editor) {
  editor.normalize().normalize()
}

const value = (
  <value>
    <document>
      {Array.from(Array(10)).map((v, i) => (
        <quote>
          <paragraph>
            <paragraph>
              <inline type="link">link text</inline>
            </paragraph>
            <paragraph />
            <paragraph>
              <paragraph>
                <inline type="link">link text</inline>
                <text> more text after</text>
              </paragraph>
            </paragraph>
          </paragraph>
        </quote>
      ))}
    </document>
  </value>
)

module.exports.input = function() {
  return new Editor({ value }, { normalize: false })
}
