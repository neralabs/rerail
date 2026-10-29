/** @jsx h */

import h from '../../../helpers/h'

export default function(editor) {
  editor.delete()
}

export const input = (
  <value>
    <document>
      <paragraph>
        <text />
        <inline type="link">
          wo<anchor />rd
        </inline>
        <text />
      </paragraph>
      <paragraph>
        <text />
        <inline type="link">
          an<focus />other
        </inline>
        <text />
      </paragraph>
    </document>
  </value>
)

export const output = (
  <value>
    <document>
      <paragraph>
        <text />
        <inline type="link">wo</inline>
        <text />
        <inline type="link">
          <cursor />other
        </inline>
        <text />
      </paragraph>
    </document>
  </value>
)
