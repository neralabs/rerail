import React, { Component } from "react";
import ReactDOM from "react-dom";
import PropTypes from "prop-types";
import ModalPortal from "./ModalPortal";
import * as ariaAppHider from "../helpers/ariaAppHider";
import SafeHTMLElement, { canUseDOM } from "../helpers/safeHTMLElement";

export const portalClassName = "ReactModalPortal";
export const bodyOpenClassName = "ReactModal__Body--open";

function getParentElement(parentSelector) {
  return parentSelector();
}

export default class Modal extends Component {
  static setAppElement(element) {
    ariaAppHider.setElement(element);
  }

  /* eslint-disable react/no-unused-prop-types */
  static propTypes = {
    isOpen: PropTypes.bool.isRequired,
    style: PropTypes.shape({
      content: PropTypes.object,
      overlay: PropTypes.object
    }),
    portalClassName: PropTypes.string,
    bodyOpenClassName: PropTypes.string,
    className: PropTypes.oneOfType([PropTypes.string, PropTypes.object]),
    overlayClassName: PropTypes.oneOfType([PropTypes.string, PropTypes.object]),
    appElement: PropTypes.instanceOf(SafeHTMLElement),
    onAfterOpen: PropTypes.func,
    onRequestClose: PropTypes.func,
    closeTimeoutMS: PropTypes.number,
    ariaHideApp: PropTypes.bool,
    shouldFocusAfter: PropTypes.bool,
    shouldCloseOnOverlayClick: PropTypes.bool,
    parentSelector: PropTypes.func,
    aria: PropTypes.object,
    role: PropTypes.string,
    contentLabel: PropTypes.string
  };
  /* eslint-enable react/no-unused-prop-types */

  static defaultProps = {
    isOpen: false,
    portalClassName,
    bodyOpenClassName,
    ariaHideApp: true,
    closeTimeoutMS: 0,
    shouldFocusAfterRender: true,
    shouldCloseOnOverlayClick: true,
    parentSelector() {
      return document.body;
    }
  };

  static defaultStyles = {
    overlay: {
      position: "fixed",
      top: 0,
      left: 0,
      right: 0,
      bottom: 0,
      backgroundColor: "rgba(255, 255, 255, 0.75)"
    },
    content: {
      position: "absolute",
      top: "40px",
      left: "40px",
      right: "40px",
      bottom: "40px",
      border: "1px solid #ccc",
      background: "#fff",
      overflow: "auto",
      WebkitOverflowScrolling: "touch",
      borderRadius: "4px",
      outline: "none",
      padding: "20px"
    }
  };

  componentDidMount() {
    if (!canUseDOM) return;

    this.node.className = this.props.portalClassName;

    const parent = getParentElement(this.props.parentSelector);
    parent.appendChild(this.node);
  }

  getSnapshotBeforeUpdate(prevProps) {
    const prevParent = getParentElement(prevProps.parentSelector);
    const nextParent = getParentElement(this.props.parentSelector);
    return { prevParent, nextParent };
  }

  componentDidUpdate(prevProps, _, snapshot) {
    if (!canUseDOM) return;
    const { isOpen, portalClassName } = this.props;

    if (prevProps.portalClassName !== portalClassName) {
      this.node.className = portalClassName;
    }

    // Stop unnecessary DOM moves if modal is remaining closed
    if (!prevProps.isOpen && !isOpen) return;

    const { prevParent, nextParent } = snapshot;
    if (nextParent !== prevParent) {
      prevParent.removeChild(this.node);
      nextParent.appendChild(this.node);
    }
  }

  componentWillUnmount() {
    if (!canUseDOM || !this.node || !this.portal) return;

    const state = this.portal.state;
    const now = Date.now();
    const closesAt =
      state.isOpen &&
      this.props.closeTimeoutMS &&
      (state.closesAt || now + this.props.closeTimeoutMS);

    if (closesAt) {
      if (!state.beforeClose) {
        this.portal.closeWithTimeout();
      }

      setTimeout(this.removePortal, closesAt - now);
    } else {
      this.removePortal();
    }
  }

  removePortal = () => {
    const parent = getParentElement(this.props.parentSelector);
    parent.removeChild(this.node);
  };

  portalRef = ref => {
    this.portal = ref;
  };

  render() {
    if (!canUseDOM) {
      return null;
    }

    if (!this.node) {
      this.node = document.createElement("div");
    }

    return ReactDOM.createPortal(
      <ModalPortal
        ref={this.portalRef}
        defaultStyles={Modal.defaultStyles}
        {...this.props}
      />,
      this.node
    );
  }
}
