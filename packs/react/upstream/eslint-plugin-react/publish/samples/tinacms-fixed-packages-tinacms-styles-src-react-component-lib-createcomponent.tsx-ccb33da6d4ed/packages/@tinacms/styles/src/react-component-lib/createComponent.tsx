import React from 'react';
import {
  attachEventProps,
  createForwardRef,
  dashToPascalCase,
  isCoveredByReact,
} from './utils/index';

interface IonicReactInternalProps<ElementType> extends React.HTMLAttributes<ElementType> {
  forwardedRef?: React.Ref<ElementType>;
  ref?: React.Ref<any>;
}

export const createReactComponent = <PropType, ElementType>(tagName: string) => {
  const displayName = dashToPascalCase(tagName);
  const ReactComponent = class extends React.Component<IonicReactInternalProps<ElementType>> {
    constructor(props: IonicReactInternalProps<ElementType>) {
      super(props);
    }

    private componentEl: HTMLElement | null = null;

    private setComponentElRef = (element: HTMLElement | null) => {
      this.componentEl = element;
      const { forwardedRef } = this.props;
      // The rendered `tagName` element is the ElementType the caller's ref expects.
      const forwarded = element as unknown as ElementType | null;
      if (typeof forwardedRef === 'function') {
        forwardedRef(forwarded);
      } else if (forwardedRef) {
        (forwardedRef as React.MutableRefObject<ElementType | null>).current = forwarded;
      }
    };

    componentDidMount() {
      this.componentDidUpdate(this.props);
    }

    componentDidUpdate(prevProps: IonicReactInternalProps<ElementType>) {
      if (this.componentEl) {
        attachEventProps(this.componentEl, this.props, prevProps);
      }
    }

    render() {
      const { children, forwardedRef, style, className, ref, ...cProps } = this.props;

      const propsToPass = Object.keys(cProps).reduce((acc, name) => {
        if (name.indexOf('on') === 0 && name[2] === name[2].toUpperCase()) {
          const eventName = name.substring(2).toLowerCase();
          if (isCoveredByReact(eventName)) {
            (acc as any)[name] = (cProps as any)[name];
          }
        }
        return acc;
      }, {});

      const newProps: any = {
        ...propsToPass,
        ref: this.setComponentElRef,
        style,
        className,
      };

      return React.createElement(tagName, newProps, children);
    }

    static get displayName() {
      return displayName;
    }
  };
  return createForwardRef<PropType, ElementType>(ReactComponent, displayName);
};
