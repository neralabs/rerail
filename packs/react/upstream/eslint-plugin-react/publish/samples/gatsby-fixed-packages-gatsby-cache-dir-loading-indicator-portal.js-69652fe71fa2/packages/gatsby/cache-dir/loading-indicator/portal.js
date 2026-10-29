import * as React from "react"
import { createPortal } from "react-dom"

// this is `fast-refresh-overlay/portal` ported to class component
// because we don't have guarantee that query on demand users will use
// react version that supports hooks
// TO-DO: consolidate both portals into single shared component (need testing)
class ShadowPortal extends React.Component {
  // create the shadow host once the placeholder is attached, so we know its owner document
  mountNode = node => {
    if (!node || this.portalNode.current) {
      return
    }
    const ownerDocument = node.ownerDocument
    this.portalNode.current = ownerDocument.createElement(`gatsby-portal`)
    this.shadowNode.current = this.portalNode.current.attachShadow({
      mode: `open`,
    })
    ownerDocument.body.appendChild(this.portalNode.current)
    this.setState({ createdElement: true })
  }
  portalNode = React.createRef(null)
  shadowNode = React.createRef(null)
  state = {
    createdElement: false,
  }

  componentWillUnmount() {
    if (this.portalNode.current && this.portalNode.current.ownerDocument) {
      this.portalNode.current.ownerDocument.body.removeChild(
        this.portalNode.current
      )
    }
  }

  render() {
    return this.shadowNode.current ? (
      createPortal(this.props.children, this.shadowNode.current)
    ) : (
      <span ref={this.mountNode} />
    )
  }
}

export default ShadowPortal
