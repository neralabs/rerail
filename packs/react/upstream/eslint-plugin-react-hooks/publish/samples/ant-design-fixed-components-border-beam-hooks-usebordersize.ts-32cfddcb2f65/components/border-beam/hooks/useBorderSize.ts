import React from 'react';

import { isNumber } from '../../_util/is';
import { isSameBorderWidth } from '../util';
import type { BorderWidth } from '../util';

const DEFAULT_BORDER_WIDTH: BorderWidth = [0, 0, 0, 0];

const normalizeValue = (val: string) => {
  const size = Number.parseFloat(val);
  return isNumber(size) ? size : 0;
};

const useBorderSize = (domNode: Element | null) => {
  const borderWidth = React.useMemo<BorderWidth>(() => {
    if (!domNode) {
      return DEFAULT_BORDER_WIDTH;
    }

    const { borderTopWidth, borderRightWidth, borderBottomWidth, borderLeftWidth } =
      getComputedStyle(domNode);

    const nextBorderWidth: BorderWidth = [
      normalizeValue(borderTopWidth),
      normalizeValue(borderRightWidth),
      normalizeValue(borderBottomWidth),
      normalizeValue(borderLeftWidth),
    ];

    return isSameBorderWidth(DEFAULT_BORDER_WIDTH, nextBorderWidth)
      ? DEFAULT_BORDER_WIDTH
      : nextBorderWidth;
  }, [domNode]);

  return borderWidth;
};

export default useBorderSize;
