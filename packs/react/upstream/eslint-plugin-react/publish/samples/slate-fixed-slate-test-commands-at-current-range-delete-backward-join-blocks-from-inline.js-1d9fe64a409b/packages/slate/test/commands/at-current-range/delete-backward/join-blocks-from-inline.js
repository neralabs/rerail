/** @jsx h */

import h from '../../../helpers/h'

export default function(editor) {
  editor.deleteBackward()
}

export const input = (
  <value>
    <document>
      <paragraph>
        one<inline type="link">two</inline>
      </paragraph>
      <paragraph>
        <inline type="link">
          <cursor />three
        </inline>four
      </paragraph>
    </document>
  </value>
)

export const output = (
  <value>
    <document>
      <paragraph>
        one<inline type="link">two</inline>
        <inline type="link">
          <cursor />three
        </inline>four
      </paragraph>
    </document>
  </value>
)
